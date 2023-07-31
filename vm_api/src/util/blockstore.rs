use cid::Cid;
use fvm_ipld_blockstore::Blockstore;

/// A DynBlockstore is used to make the blockstore trait object consumable by functions that
/// accept a generic BS: Blockstore parameter rather than a dyn Blockstore
pub struct DynBlockstore<'bs>(&'bs dyn Blockstore);

impl<'bs> Blockstore for DynBlockstore<'bs> {
    fn get(&self, k: &Cid) -> anyhow::Result<Option<Vec<u8>>> {
        self.0.get(k)
    }

    fn put_keyed(&self, k: &Cid, block: &[u8]) -> anyhow::Result<()> {
        self.0.put_keyed(k, block)
    }
}

impl<'bs> DynBlockstore<'bs> {
    pub fn wrap(blockstore: &'bs dyn Blockstore) -> Self {
        Self(blockstore)
    }
}
