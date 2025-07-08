use {
    litesvm::LiteSVM,
    solana_account::{Account, ReadableAccount},
    solana_address_lookup_table_interface::instruction::{
        create_lookup_table, extend_lookup_table,
    },
    solana_instruction::{account_meta::AccountMeta, Instruction},
    solana_keypair::Keypair,
    solana_message::{
        v0::Message as MessageV0, AddressLookupTableAccount, Message, VersionedMessage as VMsg,
    },
    solana_pubkey::{pubkey, Pubkey},
    solana_rent::Rent,
    solana_signature::Signature,
    solana_signer::Signer,
    solana_transaction::{versioned::VersionedTransaction, Transaction},
    solana_system_interface::instruction::transfer,
    solana_transaction_error::TransactionError,
    std::path::PathBuf,
    serial_test::serial,
    solana_native_token::LAMPORTS_PER_SOL,
    tempfile::TempDir,
    litesvm::storage::RocksDBStore,
};

const NUM_GREETINGS: u8 = 127;

fn read_counter_program() -> Vec<u8> {
    let mut so_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    so_path.push("test_programs/target/deploy/counter.so");
    std::fs::read(so_path).unwrap()
}

// #[test]
// #[serial]
// pub fn integration_test() {
//     // std::fs::remove_dir_all("/tmp/litesvm-db").ok();
//     let mut svm = LiteSVM::new();
//     // let mut svm = LiteSVM::new_with_db_path("/tmp/litesvm-db")
//     //     .with_builtins()
//     //     .with_lamports(1_000_000u64.wrapping_mul(LAMPORTS_PER_SOL))
//     //     .with_sysvars()
//     //     .with_spl_programs();

//     let payer_kp = Keypair::new();
//     let payer_pk = payer_kp.pubkey();
//     let program_id = pubkey!("GtdambwDgHWrDJdVPBkEHGhCwokqgAoch162teUjJse2");
//     svm.add_program(program_id, &read_counter_program());
//     svm.airdrop(&payer_pk, 1000000000).unwrap();
//     let blockhash = svm.latest_blockhash();
//     let counter_address = pubkey!("J39wvrFY2AkoAUCke5347RMNk3ditxZfVidoZ7U6Fguf");
//     let _ = svm.set_account(
//         counter_address,
//         Account {
//             lamports: 5,
//             data: vec![0_u8; std::mem::size_of::<u32>()],
//             owner: program_id,
//             ..Default::default()
//         },
//     );
//     assert_eq!(
//         svm.get_account(&counter_address).unwrap().data,
//         0u32.to_le_bytes().to_vec()
//     );
//     let num_greets = 2u8;
//     for deduper in 0..num_greets {
//         let tx = make_tx(
//             program_id,
//             counter_address,
//             &payer_pk,
//             blockhash,
//             &payer_kp,
//             deduper,
//         );
//         let _ = svm.send_transaction(tx).unwrap();
//     }
//     assert_eq!(
//         svm.get_account(&counter_address).unwrap().data,
//         (num_greets as u32).to_le_bytes().to_vec()
//     );
// }

fn make_tx(
    program_id: Pubkey,
    counter_address: Pubkey,
    payer_pk: &Pubkey,
    blockhash: solana_hash::Hash,
    payer_kp: &Keypair,
    deduper: u8,
) -> Transaction {
    let msg = Message::new_with_blockhash(
        &[Instruction {
            program_id,
            accounts: vec![AccountMeta::new(counter_address, false)],
            data: vec![0, deduper],
        }],
        Some(payer_pk),
        &blockhash,
    );
    Transaction::new(&[payer_kp], msg, blockhash)
}

fn add_program(bytes: &[u8], program_id: Pubkey, pt: &mut solana_program_test::ProgramTest) {
    pt.add_account(
        program_id,
        Account {
            lamports: Rent::default().minimum_balance(bytes.len()).max(1),
            data: bytes.to_vec(),
            owner: solana_sdk_ids::bpf_loader::id(),
            executable: true,
            rent_epoch: 0,
        },
    );
}

fn counter_acc(program_id: Pubkey) -> solana_account::Account {
    Account {
        lamports: 5,
        data: vec![0_u8; std::mem::size_of::<u32>()],
        owner: program_id,
        ..Default::default()
    }
}

async fn do_program_test(program_id: Pubkey, counter_address: Pubkey) {
    let mut pt = solana_program_test::ProgramTest::default();
    add_program(&read_counter_program(), program_id, &mut pt);
    let mut ctx = pt.start_with_context().await;
    ctx.set_account(&counter_address, &counter_acc(program_id).into());
    assert_eq!(
        ctx.banks_client
            .get_account(counter_address)
            .await
            .unwrap()
            .unwrap()
            .data,
        0u32.to_le_bytes().to_vec()
    );
    assert!(ctx
        .banks_client
        .get_account(program_id)
        .await
        .unwrap()
        .is_some());

    for deduper in 0..NUM_GREETINGS {
        let tx = make_tx(
            program_id,
            counter_address,
            &ctx.payer.pubkey(),
            ctx.last_blockhash,
            &ctx.payer,
            deduper,
        );
        let tx_res = ctx
            .banks_client
            .process_transaction_with_metadata(tx)
            .await
            .unwrap();
        tx_res.result.unwrap();
    }
    let fetched = ctx
        .banks_client
        .get_account(counter_address)
        .await
        .unwrap()
        .unwrap()
        .data[0];
    assert_eq!(fetched, NUM_GREETINGS);
}

#[test]
#[serial]
fn test_rocksdb_persistence_via_direct_store() {
    // 1) 用 TempDir 自动管理生命周期
    let tmpdir = TempDir::new().expect("create tempdir");
    let path = tmpdir.path();

    // ——— 写阶段 ———
    {
        let mut svm = LiteSVM::new_with_db_path(path)
            .with_builtins()
            .with_lamports(1_000_000 * LAMPORTS_PER_SOL)
            .with_sysvars()
            .with_spl_programs();
        let payer = Keypair::new();
        let key = pubkey!("J39wvrFY2AkoAUCke5347RMNk3ditxZfVidoZ7U6Fguf");
        let program = pubkey!("GtdambwDgHWrDJdVPBkEHGhCwokqgAoch162teUjJse2");
        svm.airdrop(&payer.pubkey(), 1_000).unwrap();
        svm.set_account(
            key,
            Account {
                lamports: 7,
                data: (123u32).to_le_bytes().to_vec(),
                owner: program,
                ..Default::default()
            },
        ).unwrap();

        // 触发一次写入
        let tx = VersionedTransaction::try_new(
            VMsg::Legacy(Message::new_with_blockhash(
                &[transfer(&payer.pubkey(), &key, 0)],
                Some(&payer.pubkey()),
                &svm.latest_blockhash(),
            )),
            &[&payer],
        )
        .unwrap();
        let _ = svm.send_transaction(tx);
        // drop(svm) 释放文件锁
    }

    // ——— 读阶段 ———
    {
        {
            // a) 通过 LiteSVM 打开
            let svm2 = LiteSVM::new_with_db_path(path)
                .with_builtins()
                .with_lamports(1_000_000 * LAMPORTS_PER_SOL)
                .with_sysvars()
                .with_spl_programs();
            let acc = svm2
                .get_account(&pubkey!("J39wvrFY2AkoAUCke5347RMNk3ditxZfVidoZ7U6Fguf"))
                .unwrap();
            // **注意**：用方法访问
            assert_eq!(acc.lamports(), 7);
            assert_eq!(
                u32::from_le_bytes(acc.data()[..4].try_into().unwrap()),
                123
            );
            drop(svm2);
        }
        

        // b) 直接打开底层 RocksDBStore 再验证一次
        let store = RocksDBStore::open(path).expect("open store");
        let acc2 = store
            .get_account(&pubkey!("J39wvrFY2AkoAUCke5347RMNk3ditxZfVidoZ7U6Fguf"))
            .unwrap()
            .unwrap();
        assert_eq!(acc2.lamports(), 7);
        assert_eq!(
            u32::from_le_bytes(acc2.data()[..4].try_into().unwrap()),
            123
        );
    }
    // TempDir 离开作用域自动删除
}

// #[test]
// #[serial]
// fn test_rocksdb_persistence_via_direct_store() {
//     // 1) 用 TempDir 自动管理生命周期
//     let dir = TempDir::new().expect("tempdir");
//     let db_path = dir.path();

//     // 2) 第一阶段：写入
//     {
//         let mut svm = LiteSVM::new_with_db_path(db_path)
//             .with_builtins()
//             .with_lamports(1_000_000u64.wrapping_mul(LAMPORTS_PER_SOL))
//             .with_sysvars()
//             .with_spl_programs();
//         let payer = Keypair::new();
//         let key = pubkey!("J39wvrFY2AkoAUCke5347RMNk3ditxZfVidoZ7U6Fguf");
//         let program_id = pubkey!("GtdambwDgHWrDJdVPBkEHGhCwokqgAoch162teUjJse2");
//         svm.airdrop(&payer.pubkey(), 1_000).unwrap();
//         svm.set_account(
//             key,
//             Account {
//                 lamports: 7,
//                 data: (123u32).to_le_bytes().to_vec(),
//                 owner: program_id,
//                 ..Default::default()
//             },
//         ).unwrap();
//         // 强制一次写：用一个 no-op tx
//         let tx = VersionedTransaction::try_new(
//             VersionedMessage::Legacy(Message::new_with_blockhash(
//                 &[solana_system_interface::instruction::transfer(
//                     &payer.pubkey(),
//                     &key,
//                     0,
//                 )],
//                 Some(&payer.pubkey()),
//                 &svm.latest_blockhash(),
//             )),
//             &[&payer],
//         ).unwrap();
//         let _ = svm.send_transaction(tx);
//         // drop(svm) 触发文件句柄释放
//     }

//     // 3) 第二阶段：重开并验证
//     {
//         // a) 用 LiteSVM 重新打开，并检查
//         let svm2 = LiteSVM::new_with_db_path(db_path)
//             .with_builtins()
//             .with_lamports(1_000_000u64.wrapping_mul(LAMPORTS_PER_SOL))
//             .with_sysvars()
//             .with_spl_programs();
//         let acc = svm2.get_account(&pubkey!("J39wvrFY2AkoAUCke5347RMNk3ditxZfVidoZ7U6Fguf"))
//             .expect("account must exist");
//         assert_eq!(acc.lamports, 7);
//         assert_eq!(u32::from_le_bytes(acc.data[..4].try_into().unwrap()), 123);

//         // b) 也可以直接打开底层 RocksDBStore，做同样断言
//         let store = RocksDBStore::open(db_path).expect("open store");
//         let acc2 = store.get_account(&pubkey!("J39wvrFY2AkoAUCke5347RMNk3ditxZfVidoZ7U6Fguf"))
//             .expect("io").expect("must exist");
//         assert_eq!(acc2.lamports(), 7);
//         assert_eq!(u32::from_le_bytes(acc2.data()[..4].try_into().unwrap()), 123);
//     }

//     // TempDir 退出作用域，自动清理整个目录
// }

// #[test]
// #[serial]
// fn test_persist_counter_account() {
//     std::fs::remove_dir_all("/tmp/litesvm-db").ok();
//     {
//         // let mut svm = LiteSVM::new();
//         // 明确调用new_with_db_path保证用RocksDB持久化
//         let mut svm = LiteSVM::new_with_db_path("/tmp/litesvm-db")
//             .with_builtins()
//             .with_lamports(1_000_000u64.wrapping_mul(LAMPORTS_PER_SOL))
//             .with_sysvars()
//             .with_spl_programs();
//         let payer = Keypair::new();
//         let counter_address = pubkey!("J39wvrFY2AkoAUCke5347RMNk3ditxZfVidoZ7U6Fguf");
//         let program_id = pubkey!("GtdambwDgHWrDJdVPBkEHGhCwokqgAoch162teUjJse2");
//         let _ = svm.airdrop(&payer.pubkey(), 10_000); // 自定义模拟airdrop函数

//         svm.set_account(
//             counter_address,
//             Account {
//                 lamports: 5,
//                 data: (42u32).to_le_bytes().to_vec(),
//                 owner: program_id,
//                 ..Default::default()
//             },
//         )
//         .unwrap();

//         // 🔥 用伪交易强制触发 block commit
//         // 构造无操作的交易（转账0 lamports）
//         let tx = VersionedTransaction::try_new(
//             VersionedMessage::Legacy(Message::new_with_blockhash(
//                 &[solana_system_interface::instruction::transfer(
//                     &payer.pubkey(),
//                     &counter_address,
//                     0,
//                 )],
//                 Some(&payer.pubkey()),
//                 &svm.latest_blockhash(),
//             )),
//             &[&payer],
//         )
//         .unwrap();

//         // ✅ 触发 send_transaction 才会写入 RocksDB
//         let _ = svm.send_transaction(tx);
//     }    
// }

// #[test]
// #[serial]
// fn test_read_persisted_counter_account() {
//     // std::fs::remove_dir_all("/tmp/litesvm-db").ok();
//     // let svm = LiteSVM::new();
//     // 这里不要删除数据库目录，直接打开之前保存的数据
//     let mut svm = LiteSVM::new_with_db_path("/tmp/litesvm-db")
//         .with_builtins()
//         .with_lamports(1_000_000u64.wrapping_mul(LAMPORTS_PER_SOL))
//         .with_sysvars()
//         .with_spl_programs();
//     let counter_address = pubkey!("J39wvrFY2AkoAUCke5347RMNk3ditxZfVidoZ7U6Fguf");
//     let acc = svm.get_account(&counter_address).unwrap();
//     println!(
//         "Counter persisted value = {}",
//         u32::from_le_bytes(acc.data[..4].try_into().unwrap())
//     );
//     let value = u32::from_le_bytes(acc.data[..4].try_into().unwrap());
//     assert_eq!(value, 42); // 验证数据还在
// }


#[test]
#[serial]
fn banks_client_test() {
    let program_id = Pubkey::new_unique();

    let counter_address = Pubkey::new_unique();
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async { do_program_test(program_id, counter_address).await });
}

fn make_tx_wrong_signature(
    program_id: Pubkey,
    counter_address: Pubkey,
    payer_pk: &Pubkey,
    blockhash: solana_hash::Hash,
    payer_kp: &Keypair,
) -> Transaction {
    let msg = Message::new_with_blockhash(
        &[Instruction {
            program_id,
            accounts: vec![AccountMeta::new(counter_address, false)],
            data: vec![0, 0],
        }],
        Some(payer_pk),
        &blockhash,
    );
    let mut tx = Transaction::new(&[&payer_kp], msg, blockhash);
    tx.signatures[0] = Signature::new_unique();
    tx
}

async fn do_program_test_wrong_signature(program_id: Pubkey, counter_address: Pubkey) {
    let mut pt = solana_program_test::ProgramTest::default();
    add_program(&read_counter_program(), program_id, &mut pt);
    let mut ctx = pt.start_with_context().await;
    ctx.set_account(&counter_address, &counter_acc(program_id).into());
    assert_eq!(
        ctx.banks_client
            .get_account(counter_address)
            .await
            .unwrap()
            .unwrap()
            .data,
        0u32.to_le_bytes().to_vec()
    );
    assert!(ctx
        .banks_client
        .get_account(program_id)
        .await
        .unwrap()
        .is_some());

    let tx = make_tx_wrong_signature(
        program_id,
        counter_address,
        &ctx.payer.pubkey(),
        ctx.last_blockhash,
        &ctx.payer,
    );
    let tx_res = ctx
        .banks_client
        .process_transaction_with_metadata(tx)
        .await
        .unwrap();
    tx_res.result.unwrap();
    let fetched = ctx
        .banks_client
        .get_account(counter_address)
        .await
        .unwrap()
        .unwrap()
        .data[0];
    assert_eq!(fetched, 1);
}

/// Confirm that process_transaction_with_metadata
/// does not do sigverify.
#[test]
#[serial]
fn test_process_transaction_with_metadata_wrong_signature() {
    let program_id = Pubkey::new_unique();

    let counter_address = Pubkey::new_unique();
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async { do_program_test_wrong_signature(program_id, counter_address).await });
}

// #[test]
// #[serial]
// fn test_address_lookup_table() {
//     // std::fs::remove_dir_all("/tmp/litesvm-db").ok();
//     let mut svm = LiteSVM::new();
//     // let mut svm = LiteSVM::new_with_db_path("/tmp/litesvm-db")
//     //     .with_builtins()
//     //     .with_lamports(1_000_000u64.wrapping_mul(LAMPORTS_PER_SOL))
//     //     .with_sysvars()
//     //     .with_spl_programs();
//     let payer_kp = Keypair::new();
//     let payer_pk = payer_kp.pubkey();
//     let program_id = pubkey!("GtdambwDgHWrDJdVPBkEHGhCwokqgAoch162teUjJse2");
//     svm.add_program(program_id, &read_counter_program());
//     svm.airdrop(&payer_pk, 1000000000).unwrap();
//     let blockhash = svm.latest_blockhash();
//     let counter_address = pubkey!("J39wvrFY2AkoAUCke5347RMNk3ditxZfVidoZ7U6Fguf");
//     let _ = svm.set_account(
//         counter_address,
//         Account {
//             lamports: 5,
//             data: vec![0_u8; std::mem::size_of::<u32>()],
//             owner: program_id,
//             ..Default::default()
//         },
//     );
//     let (lookup_table_ix, lookup_table_address) = create_lookup_table(payer_pk, payer_pk, 0);
//     let extend_ix = extend_lookup_table(
//         lookup_table_address,
//         payer_pk,
//         Some(payer_pk),
//         vec![counter_address],
//     );
//     let lookup_msg = Message::new(&[lookup_table_ix, extend_ix], Some(&payer_pk));
//     let lookup_tx = Transaction::new(&[&payer_kp], lookup_msg, blockhash);
//     svm.send_transaction(lookup_tx).unwrap();
//     let alta = AddressLookupTableAccount {
//         key: lookup_table_address,
//         addresses: vec![counter_address],
//     };
//     let counter_msg = MessageV0::try_compile(
//         &payer_pk,
//         &[Instruction {
//             program_id,
//             accounts: vec![AccountMeta::new(counter_address, false)],
//             data: vec![0, 0],
//         }],
//         &[alta],
//         blockhash,
//     )
//     .unwrap();
//     let counter_tx =
//         VersionedTransaction::try_new(VersionedMessage::V0(counter_msg), &[&payer_kp]).unwrap();
//     svm.warp_to_slot(1); // can't use the lookup table in the same slot
//     svm.send_transaction(counter_tx).unwrap();
// }

// #[test]
// #[serial]
// pub fn test_nonexistent_program() {
//     let mut svm = LiteSVM::new();
//     let payer_kp = Keypair::new();
//     let payer_pk = payer_kp.pubkey();
//     let program_id = pubkey!("GtdambwDgHWrDJdVPBkEHGhCwokqgAoch162teUjJse2");
//     svm.airdrop(&payer_pk, 1000000000).unwrap();
//     let blockhash = svm.latest_blockhash();
//     let counter_address = pubkey!("J39wvrFY2AkoAUCke5347RMNk3ditxZfVidoZ7U6Fguf");
//     svm.set_account(
//         counter_address,
//         Account {
//             lamports: 5,
//             data: vec![0_u8; std::mem::size_of::<u32>()],
//             owner: program_id,
//             ..Default::default()
//         },
//     )
//     .unwrap();
//     let tx = make_tx(
//         program_id,
//         counter_address,
//         &payer_pk,
//         blockhash,
//         &payer_kp,
//         0,
//     );
//     let err = svm.send_transaction(tx).unwrap_err();
//     assert_eq!(err.err, TransactionError::InvalidProgramForExecution);
// }
