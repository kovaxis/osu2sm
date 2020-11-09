//! Create, modify and transform in-memory simfiles.

use crate::node::prelude::*;

pub use crate::node::{
    align::Align,
    filter::{Filter, FilterOp, Property},
    osuload::OsuLoad,
    pipe::Pipe,
    rate::{Rate, RateMethod},
    remap::Remap,
    simfilefix::SimfileFix,
    simfilewrite::SimfileWrite,
    simultaneous::Simultaneous,
    space::Space,
};

mod prelude {
    pub use crate::{
        node::{
            align::Align,
            filter::{Filter, FilterOp, Property},
            osuload::OsuLoad,
            pipe::Pipe,
            remap::Remap,
            simfilefix::SimfileFix,
            simfilewrite::SimfileWrite,
            simultaneous::Simultaneous,
            space::Space,
            BucketId, BucketIter, BucketKind,
        },
        prelude::*,
    };
}

mod align;
mod filter;
mod osuload;
mod pipe;
mod rate;
mod remap;
mod simfilefix;
mod simfilewrite;
mod simultaneous;
mod space;

#[derive(Clone, Default)]
struct Bucket {
    simfiles: Vec<Box<Simfile>>,
    lists: Vec<usize>,
}
impl Bucket {
    fn take_lists<'a>(
        &'a mut self,
        mut consume: impl FnMut(Vec<Box<Simfile>>) -> Result<()>,
    ) -> Result<()> {
        let mut flat_simfiles = mem::replace(&mut self.simfiles, default());
        if self.lists.is_empty() {
            return Ok(());
        }
        for start_idx in self.lists.drain(..).rev().skip(1) {
            consume(flat_simfiles.drain(start_idx..).collect())?;
        }
        consume(flat_simfiles)?;
        Ok(())
    }

    fn put_list(&mut self, list: impl IntoIterator<Item = Box<Simfile>>) {
        self.simfiles.extend(list);
        self.lists.push(self.simfiles.len());
    }
}
impl fmt::Debug for Bucket {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        struct List(usize);
        impl fmt::Debug for List {
            fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
                write!(f, "{} simfiles", self.0)
            }
        }
        let mut last_end = 0;
        write!(f, "Bucket(")?;
        f.debug_list()
            .entries(self.lists.iter().map(|&end_idx| {
                let count = end_idx - last_end;
                last_end = end_idx;
                List(count)
            }))
            .finish()?;
        write!(f, ")")?;
        Ok(())
    }
}

/// Stores simfiles while in transit.
#[derive(Debug, Default, Clone)]
pub struct SimfileStore {
    by_name: HashMap<String, Bucket>,
    globals: HashMap<String, String>,
}
impl SimfileStore {
    pub fn reset(&mut self) {
        self.by_name.clear();
        self.globals.clear();
    }

    pub fn global_set(&mut self, name: &str, value: String) {
        match self.globals.get_mut(name) {
            Some(val) => {
                *val = value;
            }
            None => {
                self.globals.insert(name.to_string(), value);
            }
        }
    }

    pub fn global_get_expect(&self, name: &str) -> Result<&str> {
        self.global_get(name)
            .ok_or(anyhow!("global \"{}\" not set", name))
    }

    pub fn global_get(&self, name: &str) -> Option<&str> {
        self.globals.get(name).map(|s| &s[..])
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
            if let Some(mut b) = self.by_name.remove(name) {
                trace!("    take bucket \"{}\" ({:?})", name, b);
                b.take_lists(|list| visit(self, list))?;
            }
        } else {
            if let Some(b) = self.by_name.get(name) {
                trace!("    get bucket \"{}\" ({:?})", name, b);
                let mut b = b.clone();
                b.take_lists(|list| visit(self, list))?;
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

    pub fn put(&mut self, bucket: &BucketId, simfiles: Vec<Box<Simfile>>) {
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
            .put_list(simfiles);
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum BucketId {
    Resolved(String, bool),
    Auto,
    Null,
    Name(String),
    Nest(Vec<ConcreteNode>),
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
            _ => panic!("node i/o bucket not resolved: {:?}", self),
        }
    }
}

pub trait Node: fmt::Debug {
    /// Must yield all `BucketIter::Input` values before all `BucketIter::Output` values.
    fn buckets_mut(&mut self) -> BucketIter;
    /// Run on all filters once before starting.
    fn prepare(&mut self) -> Result<()> {
        Ok(())
    }
    /// Run on every filters once, so that entry point filters can load simfiles.
    fn entry(
        &self,
        _sm_store: &mut SimfileStore,
        _on_bmset: &mut dyn FnMut(&mut SimfileStore) -> Result<()>,
    ) -> Result<()> {
        Ok(())
    }
    /// Run on every filter once for each simfile set.
    fn apply(&self, sm_store: &mut SimfileStore) -> Result<()>;
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

pub fn resolve_buckets(nodes: &[ConcreteNode]) -> Result<Vec<Box<dyn Node>>> {
    struct State {
        out: Vec<Box<dyn Node>>,
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
        input: Option<&str>,
        output: Option<&str>,
        nodes: &[ConcreteNode],
        chained: bool,
    ) -> Result<()> {
        let mut last_magnetic_out = input.map(str::to_string);
        let in_node_count = nodes.len();
        for (i, orig_node) in nodes.iter().enumerate() {
            let mut node = orig_node.clone().into_dyn();
            //The last node has its output automatically bound to the output
            //However, in non-chained mode the output is always bound to the parent output
            let mut magnetic_out = if !chained || i + 1 == in_node_count {
                output.map(str::to_string)
            } else {
                None
            };
            //In non-chained mode the input is always the parent input
            if !chained {
                last_magnetic_out = input.map(str::to_string);
            }
            let mut insert_idx = ctx.out.len();
            //Resolve each bucket
            for (kind, bucket) in node.buckets_mut() {
                let name = match bucket {
                    BucketId::Auto => match kind {
                        BucketKind::Input => last_magnetic_out
                            .take()
                            .ok_or_else(|| anyhow!("attempt to use input, but previous node does not output (in node {:?})", orig_node))?,
                        BucketKind::Output => magnetic_out
                            .get_or_insert_with(|| ctx.gen_unique_name())
                            .clone(),
                        BucketKind::Generic => bail!(
                            "attempt to auto-bind generic bucket (in node {})",
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
                                    .ok_or_else(|| anyhow!("attempt to use input, but previous node does not output (in node {:?})", orig_node))?;
                                let from_nested = ctx.gen_unique_name();
                                resolve_layer(ctx, Some(&into_nested), Some(&from_nested), inner_list, false)?;
                                //Evaluate the current node _after_ the nested node is evaluated
                                insert_idx = ctx.out.len();
                                from_nested
                            }
                            BucketKind::Output => {
                                let into_nested = ctx.gen_unique_name();
                                let from_nested =
                                    magnetic_out.get_or_insert_with(|| ctx.gen_unique_name());
                                resolve_layer(ctx, Some(&into_nested), Some(from_nested), inner_list, false)?;
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
                "output from previous node is not used as input (in node {:?})",
                node
            );
            last_magnetic_out = magnetic_out;
            ctx.out.insert(insert_idx, node);
        }
        Ok(())
    }
    //Process nodes and output them here
    let mut ctx = State {
        out: Vec::with_capacity(nodes.len()),
        next_id: 0,
    };
    resolve_layer(&mut ctx, None, None, nodes, true)?;
    //Optimize the last reads from each bucket, by taking the value instead of cloning it
    let mut last_reads: HashMap<String, &mut BucketId> = default();
    for node in ctx.out.iter_mut() {
        for (kind, bucket) in node.buckets_mut() {
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
    //Prepare nodes, allowing them to modify themselves
    for node in ctx.out.iter_mut() {
        node.prepare()?;
    }
    //Finally, unwrap the output
    Ok(ctx.out)
}

macro_rules! make_concrete {
    ($($node:ident,)*) => {
        #[derive(Debug, Clone, Serialize, Deserialize)]
        pub enum ConcreteNode {
            $($node($node),)*
        }
        impl ConcreteNode {
            pub fn into_dyn(self) -> Box<dyn Node> {
                match self {
                    $(
                        ConcreteNode::$node(node) => Box::new(node),
                    )*
                }
            }

            pub fn as_dyn(&self) -> &dyn Node {
                match self {
                    $(
                        ConcreteNode::$node(node) => node,
                    )*
                }
            }
        }
        $(
            impl From<$node> for ConcreteNode {
                fn from(node: $node) -> Self {
                    Self::$node(node)
                }
            }
        )*
    };
}

make_concrete!(
    Pipe,
    Filter,
    Remap,
    Simultaneous,
    Align,
    SimfileFix,
    Rate,
    Space,
    OsuLoad,
    SimfileWrite,
);
