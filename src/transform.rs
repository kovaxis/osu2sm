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
        let name = bucket.unwrap_name();
        if name.is_empty() {
            //Null bucket
            return Ok(());
        }
        if let Some(list) = self.by_name.get(name) {
            let list = list.clone();
            visit(self, list)?;
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
            return;
        }
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
    Named(String),
    Inline(Box<ConcreteTransform>),
}
impl Default for BucketId {
    fn default() -> Self {
        Self::Auto
    }
}
impl BucketId {
    #[track_caller]
    fn unwrap_name(&self) -> &str {
        match self {
            BucketId::Resolved(name, _) => &name[..],
            _ => panic!("transform i/o bucket not resolved: {:?}", self),
        }
    }
}

pub trait Transform: fmt::Debug {
    fn apply(&self, sm_store: &mut SimfileStore) -> Result<()>;
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

pub fn resolve_buckets(transforms: &mut Vec<Box<dyn Transform>>) -> Result<()> {
    let mut next_id = 0;
    let mut gen_unique_name = || {
        next_id += 1;
        format!("~{}", next_id)
    };
    let mut out_transforms = Vec::with_capacity(transforms.len());
    let mut last_magnetic_out = None;
    let in_transform_count = transforms.len();
    for (i, mut trans) in transforms.drain(..).enumerate() {
        let mut magnetic_out = if i + 1 == in_transform_count {
            Some("~out".to_string())
        } else {
            None
        };
        for (kind, bucket) in trans.buckets_mut() {
            let name = match bucket {
                BucketId::Auto => match kind {
                    BucketKind::Input => last_magnetic_out.as_deref().unwrap_or("~in").to_string(),
                    BucketKind::Output => magnetic_out
                        .get_or_insert_with(&mut gen_unique_name)
                        .clone(),
                    BucketKind::Generic => bail!(
                        "    attempt to auto-bind generic bucket (in transform {})",
                        i + 1
                    ),
                },
                BucketId::Named(name) => {
                    ensure!(
                        !name.starts_with("~"),
                        "bucket names starting with '~' are reserved and cannot be used"
                    );
                    mem::replace(name, String::new())
                }
                BucketId::Inline(trans) => todo!("inline transforms"),
                BucketId::Null => "".to_string(),
                BucketId::Resolved(..) => bail!("resolved buckets cannot be used directly"),
            };
            *bucket = BucketId::Resolved(name, false);
        }
        last_magnetic_out = magnetic_out;
        out_transforms.push(trans);
    }
    *transforms = out_transforms;
    Ok(())
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
