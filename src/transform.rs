//! Transformations on in-memory simfiles.

use crate::transform::prelude::*;

pub use crate::transform::{
    filter::Filter, pipe::Pipe, remap::Remap, simfilefix::SimfileFix, simultaneous::Simultaneous,
    snap::Snap,
};

mod prelude {
    pub use crate::{
        prelude::*,
        transform::{
            filter::Filter, pipe::Pipe, remap::Remap, simfilefix::SimfileFix,
            simultaneous::Simultaneous, snap::Snap, BucketId, BucketIter, BucketKind,
        },
    };
}

mod filter;
mod pipe;
mod remap;
mod simfilefix;
mod simultaneous;
mod snap;

/// Stores simfiles while they are being transformed.
#[derive(Debug, Default, Clone)]
pub struct SimfileStore {
    by_name: HashMap<String, Vec<Box<Simfile>>>,
}
impl SimfileStore {
    pub fn reset(&mut self, input: Vec<Box<Simfile>>) {
        self.by_name.clear();
        self.by_name.insert("~in".to_string(), input);
    }

    pub fn take_output(&mut self) -> Vec<Box<Simfile>> {
        self.by_name.remove("~out").unwrap_or_default()
    }

    pub fn get<F>(&mut self, bucket: &BucketId, mut visit: F) -> Result<()>
    where
        F: FnMut(&mut SimfileStore, Vec<Box<Simfile>>) -> Result<()>,
    {
        let (name, take) = bucket.unwrap_resolved();
        if name.is_empty() {
            //Null bucket
            trace!("    get null bucket");
            return Ok(());
        }
        if take {
            if let Some(list) = self.by_name.remove(name) {
                trace!("    take bucket \"{}\" ({} simfiles)", name, list.len());
                visit(self, list)?;
            }
        } else {
            if let Some(list) = self.by_name.get(name) {
                let list = list.clone();
                trace!("    get bucket \"{}\" ({} simfiles)", name, list.len());
                visit(self, list)?;
            }
        }
        Ok(())
    }

    pub fn get_each<F>(&mut self, bucket: &BucketId, mut visit: F) -> Result<()>
    where
        F: FnMut(&mut SimfileStore, Box<Simfile>) -> Result<()>,
    {
        self.get(bucket, |store, list| {
            for sm in list {
                visit(store, sm)?;
            }
            Ok(())
        })
    }

    pub fn put(&mut self, bucket: &BucketId, mut simfiles: Vec<Box<Simfile>>) {
        let name = bucket.unwrap_name();
        if name.is_empty() {
            //Null bucket
            trace!("    put {} simfiles in null bucket", simfiles.len());
            return;
        }
        trace!("    put {} simfiles in bucket \"{}\"", simfiles.len(), name);
        self.by_name
            .entry(name.to_string())
            .or_default()
            .append(&mut simfiles);
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum BucketId {
    Resolved(String, bool),
    Auto,
    Null,
    Name(String),
    Nest(Vec<ConcreteTransform>),
}
impl Default for BucketId {
    fn default() -> Self {
        Self::Auto
    }
}
impl BucketId {
    #[track_caller]
    fn unwrap_name(&self) -> &str {
        self.unwrap_resolved().0
    }

    #[track_caller]
    fn unwrap_resolved(&self) -> (&str, bool) {
        match self {
            BucketId::Resolved(name, take) => (&name[..], *take),
            _ => panic!("transform i/o bucket not resolved: {:?}", self),
        }
    }
}

pub trait Transform: fmt::Debug {
    fn apply(&self, sm_store: &mut SimfileStore) -> Result<()>;
    /// Must yield all `BucketIter::Input` values before all `BucketIter::Output` values.
    fn buckets_mut(&mut self) -> BucketIter;
}

pub type BucketIter<'a> = Box<dyn 'a + Iterator<Item = (BucketKind, &'a mut BucketId)>>;

pub enum BucketKind {
    Generic,
    Input,
    Output,
}
impl BucketKind {
    pub fn is_input(&self) -> bool {
        match self {
            Self::Input => true,
            _ => false,
        }
    }
    pub fn is_output(&self) -> bool {
        match self {
            Self::Output => true,
            _ => false,
        }
    }
}

pub fn resolve_buckets(transforms: &[ConcreteTransform]) -> Result<Vec<Box<dyn Transform>>> {
    struct State {
        out: Vec<Box<dyn Transform>>,
        next_id: u32,
    }
    impl State {
        fn gen_unique_name(&mut self) -> String {
            self.next_id += 1;
            format!("~{}", self.next_id)
        }
    }
    //Keep track of the last auto-output, to bind it to any auto-input
    fn resolve_layer(
        ctx: &mut State,
        input: &str,
        output: &str,
        transforms: &[ConcreteTransform],
        chained: bool,
    ) -> Result<()> {
        let mut last_magnetic_out = Some(input.to_string());
        let in_transform_count = transforms.len();
        for (i, orig_trans) in transforms.iter().enumerate() {
            let mut trans = orig_trans.clone().into_dyn();
            //The last transform has its output automatically bound to the output
            //However, in non-chained mode the output is always bound to the parent output
            let mut magnetic_out = if !chained || i + 1 == in_transform_count {
                Some(output.to_string())
            } else {
                None
            };
            //In non-chained mode the input is always the parent input
            if !chained {
                last_magnetic_out = Some(input.to_string());
            }
            let mut insert_idx = ctx.out.len();
            //Resolve each bucket
            for (kind, bucket) in trans.buckets_mut() {
                let name = match bucket {
                    BucketId::Auto => match kind {
                        BucketKind::Input => last_magnetic_out
                            .take()
                            .ok_or_else(|| anyhow!("attempt to use input, but previous transform does not output (in transform {:?})", orig_trans))?,
                        BucketKind::Output => magnetic_out
                            .get_or_insert_with(|| ctx.gen_unique_name())
                            .clone(),
                        BucketKind::Generic => bail!(
                            "attempt to auto-bind generic bucket (in transform {})",
                            i + 1
                        ),
                    },
                    BucketId::Name(name) => {
                        ensure!(
                            !name.starts_with("~"),
                            "bucket names starting with '~' are reserved and cannot be used"
                        );
                        mem::replace(name, String::new())
                    }
                    BucketId::Nest(inner_list) => {
                        match kind {
                            BucketKind::Input => {
                                let into_nested = last_magnetic_out
                                    .take()
                                    .ok_or_else(|| anyhow!("attempt to use input, but previous transform does not output (in transform {:?})", orig_trans))?;
                                let from_nested = ctx.gen_unique_name();
                                resolve_layer(ctx, &into_nested, &from_nested, inner_list, false)?;
                                //Evaluate the current transform _after_ the nested transform is
                                //evaluated
                                insert_idx = ctx.out.len();
                                from_nested
                            }
                            BucketKind::Output => {
                                let into_nested = ctx.gen_unique_name();
                                let from_nested =
                                    magnetic_out.get_or_insert_with(|| ctx.gen_unique_name());
                                resolve_layer(ctx, &into_nested, from_nested, inner_list, false)?;
                                into_nested
                            }
                            BucketKind::Generic => bail!("cannot use generic buckets with `Nest`"),
                        }
                    },
                    BucketId::Null => "".to_string(),
                    BucketId::Resolved(..) => bail!("resolved buckets cannot be used directly"),
                };
                *bucket = BucketId::Resolved(name, false);
            }
            //Bookkeeping
            ensure!(
                last_magnetic_out.is_none() || i == 0,
                "output from previous transform is not used as input (in transform {:?})",
                trans
            );
            last_magnetic_out = magnetic_out;
            ctx.out.insert(insert_idx, trans);
        }
        Ok(())
    }
    //Process transforms and output them here
    let mut ctx = State {
        out: Vec::with_capacity(transforms.len()),
        next_id: 0,
    };
    resolve_layer(&mut ctx, "~in", "~out", transforms, true)?;
    //Optimize the last reads from each bucket, by taking the value instead of cloning it
    let mut last_reads: HashMap<String, &mut BucketId> = default();
    for trans in ctx.out.iter_mut() {
        for (kind, bucket) in trans.buckets_mut() {
            if kind.is_input() {
                last_reads.insert(bucket.unwrap_name().to_string(), bucket);
            }
        }
    }
    for (_name, bucket) in last_reads {
        match bucket {
            BucketId::Resolved(_name, take) => {
                *take = true;
            }
            _ => panic!("unresolved bucket"),
        }
    }
    //Finally, unwrap the output
    Ok(ctx.out)
}

macro_rules! make_concrete {
    ($($trans:ident,)*) => {
        #[derive(Debug, Clone, Serialize, Deserialize)]
        pub enum ConcreteTransform {
            $($trans($trans),)*
        }
        impl ConcreteTransform {
            pub fn into_dyn(self) -> Box<dyn Transform> {
                match self {
                    $(
                        ConcreteTransform::$trans(trans) => Box::new(trans),
                    )*
                }
            }

            pub fn as_dyn(&self) -> &dyn Transform {
                match self {
                    $(
                        ConcreteTransform::$trans(trans) => trans,
                    )*
                }
            }
        }
        $(
            impl From<$trans> for ConcreteTransform {
                fn from(trans: $trans) -> Self {
                    Self::$trans(trans)
                }
            }
        )*
    };
}

make_concrete!(Pipe, Filter, Remap, Simultaneous, Snap, SimfileFix,);
