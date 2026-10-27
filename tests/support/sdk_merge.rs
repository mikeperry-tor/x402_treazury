// Actual SDK tree-merge regression through public traits, with no network requests.
use incrementalmerkletree::Hashable;
use incrementalmerkletree::{Address, Level};
use pepper_sync::{
    error::{ShardTreeOperation, SyncError},
    keys::transparent::TransparentAddressId,
    sync::{ScanPriority, ScanRange},
    wallet::{
        ShardTrees, SyncState,
        traits::{LocatedTreeData, SyncShardTrees, SyncWallet},
    },
};
use shardtree::store::ShardStore;
use shardtree::{
    LocatedPrunableTree, RetentionFlags,
    error::{InsertionError, ShardTreeError},
};
use std::collections::{BTreeMap, HashMap};
use tokio::sync::mpsc;
use zcash_keys::keys::UnifiedFullViewingKey;
use zcash_protocol::{PoolType, consensus::BlockHeight};
use zip32::{AccountId, DiversifierIndex};

struct TreeWallet {
    trees: ShardTrees,
    sync: SyncState,
    addresses: BTreeMap<TransparentAddressId, String>,
}
impl SyncWallet for TreeWallet {
    type Error = std::convert::Infallible;
    fn get_birthday(&self) -> Result<BlockHeight, Self::Error> {
        Ok(0u32.into())
    }
    fn get_sync_state(&self) -> Result<&SyncState, Self::Error> {
        Ok(&self.sync)
    }
    fn get_sync_state_mut(&mut self) -> Result<&mut SyncState, Self::Error> {
        Ok(&mut self.sync)
    }
    fn get_unified_full_viewing_keys(
        &self,
    ) -> Result<HashMap<AccountId, UnifiedFullViewingKey>, Self::Error> {
        Ok(HashMap::new())
    }
    fn add_orchard_address(
        &mut self,
        _: AccountId,
        _: orchard::Address,
        _: DiversifierIndex,
    ) -> Result<(), Self::Error> {
        unreachable!("merge fixture does not derive addresses")
    }
    fn add_sapling_address(
        &mut self,
        _: AccountId,
        _: sapling_crypto::PaymentAddress,
        _: DiversifierIndex,
    ) -> Result<(), Self::Error> {
        unreachable!("merge fixture does not derive addresses")
    }
    fn get_transparent_addresses(
        &self,
    ) -> Result<&BTreeMap<TransparentAddressId, String>, Self::Error> {
        Ok(&self.addresses)
    }
    fn get_transparent_addresses_mut(
        &mut self,
    ) -> Result<&mut BTreeMap<TransparentAddressId, String>, Self::Error> {
        Ok(&mut self.addresses)
    }
}
impl SyncShardTrees for TreeWallet {
    fn get_shard_trees(&self) -> Result<&ShardTrees, Self::Error> {
        Ok(&self.trees)
    }
    fn get_shard_trees_mut(&mut self) -> Result<&mut ShardTrees, Self::Error> {
        Ok(&mut self.trees)
    }
}

fn conflict_pair<H: Hashable + Clone + PartialEq, const DEPTH: u8, const SHARD_HEIGHT: u8>(
    _: &shardtree::ShardTree<
        shardtree::store::memory::MemoryShardStore<H, BlockHeight>,
        DEPTH,
        SHARD_HEIGHT,
    >,
) -> (LocatedPrunableTree<H>, LocatedTreeData<H>) {
    let address = Address::from_parts(Level::from(SHARD_HEIGHT), 0);
    let existing = H::empty_leaf();
    let incoming = H::combine(Level::from(0), &existing, &existing);
    assert!(existing != incoming);
    (
        LocatedPrunableTree::with_root_value(address, (existing, RetentionFlags::EPHEMERAL)),
        LocatedTreeData::from_parts(
            LocatedPrunableTree::with_root_value(address, (incoming, RetentionFlags::EPHEMERAL)),
            BTreeMap::new(),
        ),
    )
}

#[tokio::test]
async fn scan_merge_conflict_preserves_pool_context() {
    for pool in [PoolType::SAPLING, PoolType::ORCHARD, PoolType::IRONWOOD] {
        let mut wallet = TreeWallet {
            trees: ShardTrees::new(),
            sync: SyncState::new(),
            addresses: BTreeMap::new(),
        };
        let mut sapling = vec![];
        let mut orchard = vec![];
        let mut ironwood = vec![];
        let trees = wallet.get_shard_trees_mut().unwrap();
        match pool {
            PoolType::SAPLING => {
                let (existing, incoming) = conflict_pair(&trees.sapling);
                trees.sapling.store_mut().put_shard(existing).unwrap();
                sapling.push(incoming);
            }
            PoolType::ORCHARD => {
                let (existing, incoming) = conflict_pair(&trees.orchard);
                trees.orchard.store_mut().put_shard(existing).unwrap();
                orchard.push(incoming);
            }
            PoolType::IRONWOOD => {
                let (existing, incoming) = conflict_pair(&trees.ironwood);
                trees.ironwood.store_mut().put_shard(existing).unwrap();
                ironwood.push(incoming);
            }
            _ => unreachable!(),
        }
        let (sender, mut receiver) = mpsc::unbounded_channel();
        // Empty checkpoint range isolates the actual tree merge without RPCs.
        let range = ScanRange::from_parts(1u32.into()..1u32.into(), ScanPriority::Historic);
        let error = wallet
            .update_shard_trees(
                &zcash_protocol::consensus::MAIN_NETWORK,
                sender,
                &range,
                100_000u32.into(),
                None,
                sapling,
                orchard,
                ironwood,
            )
            .await
            .unwrap_err();
        assert!(matches!(error, SyncError::ShardTreeContext {
            pool: actual,
            operation: ShardTreeOperation::ScanMerge,
            error: ShardTreeError::Insert(InsertionError::Conflict(_)),
        } if actual == pool));
        assert!(!error.recommend_same_server());
        assert!(receiver.try_recv().is_err());
    }
}
