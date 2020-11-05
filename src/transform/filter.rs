use crate::transform::prelude::*;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Filter {
    pub from: BucketId,
    pub into: BucketId,
    pub whitelist: Vec<Gamemode>,
    pub blacklist: Vec<Gamemode>,
}

impl Transform for Filter {
    fn apply(&self, store: &mut SimfileStore) -> Result<()> {
        store.get(&self.from, |store, mut list| {
            list.retain(|sm| {
                !self.blacklist.contains(&sm.gamemode)
                    && (self.whitelist.is_empty() || self.whitelist.contains(&sm.gamemode))
            });
            store.put(&self.into, list);
            Ok(())
        })
    }
    fn buckets_mut<'a>(&'a mut self) -> BucketIter<'a> {
        Box::new(
            iter::once((BucketKind::Input, &mut self.from))
                .chain(iter::once((BucketKind::Output, &mut self.into))),
        )
    }
}
