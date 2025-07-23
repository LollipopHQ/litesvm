// use solana_sdk::{
//     account::AccountSharedData,
//     pubkey::Pubkey,
// };
use solana_account::AccountSharedData;
use std::sync::Arc; // 导入 Arc
use {
    bincode, num_cpus,
    rocksdb::{ColumnFamilyDescriptor, Options, DB},
    solana_pubkey::Pubkey,
    std::path::Path,
    thiserror::Error,
};

#[derive(Error, Debug)]
pub enum StoreError {
    #[error("RocksDB operation failed: {0}")]
    RocksDB(#[from] rocksdb::Error),
    #[error("Serialization error: {0}")]
    Serialization(#[from] bincode::Error),
}

type Result<T> = std::result::Result<T, StoreError>;

#[repr(u8)]
enum KeyPrefix {
    Account = 0x01, // 存储完整账户
    ProgramData = 0x02, // 存储程序数据
                    // 可以添加更多数据类型...
}

pub enum DbKey {
    Account(Pubkey),     // 账户键
    ProgramData(Pubkey), // 程序数据键
}

impl DbKey {
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(33);
        match self {
            Self::Account(pubkey) => {
                bytes.push(KeyPrefix::Account as u8);
                bytes.extend_from_slice(pubkey.as_ref());
            }
            Self::ProgramData(pubkey) => {
                bytes.push(KeyPrefix::ProgramData as u8);
                bytes.extend_from_slice(pubkey.as_ref());
            }
        }
        bytes
    }
}

pub struct RocksDBStore {
    db: Arc<DB>, // 使用 Arc 共享数据库实例
}

impl RocksDBStore {
    pub fn open(path: &Path) -> Result<Self> {
        let mut opts = Options::default();
        opts.create_if_missing(true);
        opts.create_missing_column_families(true);
        opts.set_compression_type(rocksdb::DBCompressionType::Zstd);
        opts.set_max_open_files(1024);
        opts.increase_parallelism(num_cpus::get() as i32);

        // 定义列族
        let cfs = vec!["accounts", "program_data"];
        let cf_descriptors: Vec<_> = cfs
            .iter()
            .map(|name| ColumnFamilyDescriptor::new(*name, Options::default()))
            .collect();

        let db = DB::open_cf_descriptors(&opts, path, cf_descriptors)?;

        Ok(Self {
            db: Arc::new(db), // 使用 Arc 包装数据库
        })
    }

    // 获取列族句柄的辅助方法
    fn accounts_cf(&self) -> &rocksdb::ColumnFamily {
        self.db
            .cf_handle("accounts")
            .expect("Accounts column family not found")
    }

    fn program_data_cf(&self) -> &rocksdb::ColumnFamily {
        self.db
            .cf_handle("program_data")
            .expect("Program data column family not found")
    }

    /// 获取账户数据
    pub fn get_account(&self, pubkey: &Pubkey) -> Result<Option<AccountSharedData>> {
        let key = DbKey::Account(*pubkey).to_bytes();
        match self.db.get_cf(self.accounts_cf(), &key)? {
            Some(data) => bincode::deserialize(&data).map(Some).map_err(Into::into),
            None => Ok(None),
        }
    }

    /// 存储账户数据
    pub fn put_account(&self, pubkey: &Pubkey, account: &AccountSharedData) -> Result<()> {
        let key = DbKey::Account(*pubkey).to_bytes();
        let value = bincode::serialize(account)?;
        self.db.put_cf(self.accounts_cf(), &key, &value)?;
        Ok(())
    }

    /// 批量存储账户数据
    pub fn put_accounts(&self, accounts: &[(Pubkey, AccountSharedData)]) -> Result<()> {
        let mut batch = rocksdb::WriteBatch::default();
        let accounts_cf = self.accounts_cf();

        for (pubkey, account) in accounts {
            let key = DbKey::Account(*pubkey).to_bytes();
            let value = bincode::serialize(account)?;
            batch.put_cf(accounts_cf, &key, &value);
        }

        self.db.write(batch)?;
        Ok(())
    }

    /// 获取程序数据
    pub fn get_program_data(&self, pubkey: &Pubkey) -> Result<Option<Vec<u8>>> {
        let key = DbKey::ProgramData(*pubkey).to_bytes();
        self.db
            .get_cf(self.program_data_cf(), &key)
            .map_err(Into::into)
    }

    /// 存储程序数据
    pub fn put_program_data(&self, pubkey: &Pubkey, data: &[u8]) -> Result<()> {
        let key = DbKey::ProgramData(*pubkey).to_bytes();
        self.db.put_cf(self.program_data_cf(), &key, data)?;
        Ok(())
    }

    // 创建检查点
    pub fn create_checkpoint<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        let checkpoint = rocksdb::checkpoint::Checkpoint::new(&self.db)?;
        checkpoint.create_checkpoint(path)?;
        Ok(())
    }

    // 克隆数据库引用
    pub fn clone_db(&self) -> Arc<DB> {
        self.db.clone()
    }
}
