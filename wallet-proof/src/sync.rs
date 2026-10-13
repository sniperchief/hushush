//! Chain sync via the upstream `zcash_client_backend::sync::run` driver.

use anyhow::anyhow;
use zcash_client_backend::sync;

use crate::{NETWORK, blockcache::MemBlockCache, remote::Client, wallet::Db};

const BATCH_SIZE: u32 = 1000;

pub async fn run(client: &mut Client, db: &mut Db) -> anyhow::Result<()> {
    let cache = MemBlockCache::default();
    sync::run(client, &NETWORK, &cache, db, BATCH_SIZE)
        .await
        .map_err(|e| anyhow!("sync failed: {e}"))
}
