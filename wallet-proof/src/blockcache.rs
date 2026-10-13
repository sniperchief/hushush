//! In-memory compact block cache for `zcash_client_backend::sync::run`.
//!
//! Compact blocks are only needed transiently while scanning, so we do not
//! persist them; durable wallet state lives entirely in the sqlite wallet DB.

use std::collections::BTreeMap;
use std::sync::Mutex;

use async_trait::async_trait;
use zcash_client_backend::data_api::chain::{BlockCache, BlockSource, error::Error as ChainError};
use zcash_client_backend::data_api::scanning::ScanRange;
use zcash_client_backend::proto::compact_formats::CompactBlock;
use zcash_protocol::consensus::BlockHeight;

#[derive(Default)]
pub struct MemBlockCache {
    blocks: Mutex<BTreeMap<u32, CompactBlock>>,
}

#[derive(Debug)]
pub struct CacheError(pub String);

impl std::fmt::Display for CacheError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "block cache error: {}", self.0)
    }
}
impl std::error::Error for CacheError {}

fn height_of(b: &CompactBlock) -> u32 {
    u32::try_from(b.height).expect("block height fits in u32")
}

impl BlockSource for MemBlockCache {
    type Error = CacheError;

    fn with_blocks<F, WalletErrT>(
        &self,
        from_height: Option<BlockHeight>,
        limit: Option<usize>,
        mut with_block: F,
    ) -> Result<(), ChainError<WalletErrT, Self::Error>>
    where
        F: FnMut(CompactBlock) -> Result<(), ChainError<WalletErrT, Self::Error>>,
    {
        let start = from_height.map_or(0, u32::from);
        let blocks: Vec<CompactBlock> = {
            let guard = self.blocks.lock().expect("lock poisoned");
            guard
                .range(start..)
                .take(limit.unwrap_or(usize::MAX))
                .map(|(_, b)| b.clone())
                .collect()
        };
        for b in blocks {
            with_block(b)?;
        }
        Ok(())
    }
}

#[async_trait]
impl BlockCache for MemBlockCache {
    fn get_tip_height(&self, range: Option<&ScanRange>) -> Result<Option<BlockHeight>, CacheError> {
        let guard = self.blocks.lock().expect("lock poisoned");
        let tip = match range {
            Some(r) => guard
                .range(u32::from(r.block_range().start)..u32::from(r.block_range().end))
                .next_back()
                .map(|(h, _)| *h),
            None => guard.keys().next_back().copied(),
        };
        Ok(tip.map(BlockHeight::from_u32))
    }

    async fn read(&self, range: &ScanRange) -> Result<Vec<CompactBlock>, CacheError> {
        let guard = self.blocks.lock().expect("lock poisoned");
        Ok(guard
            .range(u32::from(range.block_range().start)..u32::from(range.block_range().end))
            .map(|(_, b)| b.clone())
            .collect())
    }

    async fn insert(&self, compact_blocks: Vec<CompactBlock>) -> Result<(), CacheError> {
        let mut guard = self.blocks.lock().expect("lock poisoned");
        for b in compact_blocks {
            guard.insert(height_of(&b), b);
        }
        Ok(())
    }

    async fn delete(&self, range: ScanRange) -> Result<(), CacheError> {
        let mut guard = self.blocks.lock().expect("lock poisoned");
        let (s, e) = (
            u32::from(range.block_range().start),
            u32::from(range.block_range().end),
        );
        guard.retain(|h, _| !(s..e).contains(h));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zcash_client_backend::data_api::scanning::ScanPriority;

    fn block(h: u64) -> CompactBlock {
        CompactBlock {
            height: h,
            ..Default::default()
        }
    }

    fn range(s: u32, e: u32) -> ScanRange {
        ScanRange::from_parts(
            BlockHeight::from_u32(s)..BlockHeight::from_u32(e),
            ScanPriority::Historic,
        )
    }

    #[tokio::test]
    async fn insert_read_tip_delete() {
        let c = MemBlockCache::default();
        c.insert(vec![block(12), block(10), block(11)]).await.unwrap();
        assert_eq!(c.get_tip_height(None).unwrap(), Some(BlockHeight::from_u32(12)));
        assert_eq!(
            c.get_tip_height(Some(&range(10, 12))).unwrap(),
            Some(BlockHeight::from_u32(11))
        );
        let got: Vec<u64> = c.read(&range(10, 12)).await.unwrap().iter().map(|b| b.height).collect();
        assert_eq!(got, vec![10, 11]);
        c.delete(range(10, 12)).await.unwrap();
        assert_eq!(c.read(&range(0, 100)).await.unwrap().len(), 1);
    }

    #[test]
    fn with_blocks_respects_start_and_limit() {
        let c = MemBlockCache::default();
        {
            let mut g = c.blocks.lock().unwrap();
            for h in 1..=5 {
                g.insert(h, block(h as u64));
            }
        }
        let mut seen = vec![];
        c.with_blocks::<_, ()>(Some(BlockHeight::from_u32(2)), Some(2), |b| {
            seen.push(b.height);
            Ok(())
        })
        .unwrap();
        assert_eq!(seen, vec![2, 3]);
    }
}
