use crate::prelude::*;

mod prelude {
    pub(crate) use crate::{
        linear_map,
        node::{ConcreteNode, Node, SimfileStore},
        osufile::{self, Beatmap, TimingPoint},
        simfile::{BeatPos, ControlPoint, Difficulty, DisplayBpm, Gamemode, Note, Simfile, ToTime},
        simfile_rng, symlink_dir, symlink_file, BaseDirFinder,
    };
    pub use anyhow::{anyhow, bail, ensure, Context, Error, Result};
    pub use fxhash::{FxHashMap as HashMap, FxHashSet as HashSet};
    pub use log::{debug, error, info, trace, warn};
    pub use rand::{
        seq::{IteratorRandom, SliceRandom},
        Rng, RngCore, SeedableRng,
    };
    pub use rand_xoshiro::Xoshiro256Plus as FastRng;
    pub use serde::{Deserialize, Serialize};
    pub use std::{
        borrow::Cow,
        cell::{Cell, RefCell},
        cmp,
        convert::{TryFrom, TryInto},
        ffi::{OsStr, OsString},
        fmt::{self, Write as _},
        fs::{self, File},
        io::{self, BufRead, BufReader, BufWriter, Read, Write},
        iter, mem, ops,
        path::{Path, PathBuf},
        time::Instant,
    };
    pub use walkdir::WalkDir;
    pub fn default<T: Default>() -> T {
        T::default()
    }
    #[derive(Debug, Clone, Copy)]
    pub struct SortableFloat(pub f64);
    impl Ord for SortableFloat {
        fn cmp(&self, rhs: &Self) -> cmp::Ordering {
            self.0.partial_cmp(&rhs.0).unwrap_or_else(|| {
                if self.0.is_nan() == rhs.0.is_nan() {
                    cmp::Ordering::Equal
                } else if self.0.is_nan() {
                    cmp::Ordering::Less
                } else {
                    cmp::Ordering::Greater
                }
            })
        }
    }
    impl PartialOrd for SortableFloat {
        fn partial_cmp(&self, rhs: &Self) -> Option<cmp::Ordering> {
            Some(self.cmp(rhs))
        }
    }
    impl PartialEq for SortableFloat {
        fn eq(&self, rhs: &Self) -> bool {
            self.cmp(rhs) == cmp::Ordering::Equal
        }
    }
    impl Eq for SortableFloat {}
}

pub mod node;
pub mod osufile;
pub mod simfile;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
struct Opts {
    /// A graph of nodes to load, transform and save simfiles.
    nodes: Vec<ConcreteNode>,
    /// Whether to carry out redundant sanity checks.
    /// (Will likely error on kinda-correct, mistimed and simultaneous-slider beatmaps).
    sanity_check: bool,
    /// A logspec string (see
    /// https://https://docs.rs/flexi_logger/0.16.1/flexi_logger/struct.LogSpecification.html).
    log: String,
    /// Whether to log to a file.
    log_file: bool,
    /// Enable logging to stderr.
    log_stderr: bool,
    /// Enable logging to stdout.
    log_stdout: bool,
}
impl Default for Opts {
    fn default() -> Opts {
        Opts {
            nodes: vec![
                node::osuload::OsuLoad {
                    input: "".to_string(),
                    standard: node::osuload::OsuStd {
                        //Disable the standard parser by default
                        keycount: 0,
                        ..default()
                    },
                    ..default()
                }
                .into(),
                node::rekey::Rekey {
                    gamemode: Gamemode::DanceSingle,
                    ..default()
                }
                .into(),
                node::rate::Rate { ..default() }.into(),
                node::select::Select { ..default() }.into(),
                node::simfilewrite::SimfileWrite {
                    output: "".to_string(),
                    ..default()
                }
                .into(),
            ],
            sanity_check: false,
            log: "info".to_string(),
            log_file: true,
            log_stderr: true,
            log_stdout: false,
        }
    }
}
impl Opts {
    fn apply(&self) {
        let log_target = if self.log_file {
            flexi_logger::LogTarget::File
        } else {
            flexi_logger::LogTarget::DevNull
        };
        let log_stderr = if self.log_stderr {
            flexi_logger::Duplicate::All
        } else {
            flexi_logger::Duplicate::None
        };
        let log_stdout = if self.log_stdout {
            flexi_logger::Duplicate::All
        } else {
            flexi_logger::Duplicate::None
        };

        if let Err(err) = flexi_logger::Logger::with_str(&self.log)
            .log_target(log_target)
            .duplicate_to_stderr(log_stderr)
            .duplicate_to_stdout(log_stdout)
            .start()
        {
            eprintln!("error initializing logger: {:#}", err);
        }
    }
}

struct Ctx {
    sm_store: RefCell<SimfileStore>,
    nodes: Vec<Box<dyn Node>>,
    opts: Opts,
}

fn run_nodes(ctx: &Ctx) -> Result<()> {
    let mut store = ctx.sm_store.borrow_mut();
    for (i, node) in ctx.nodes.iter().enumerate() {
        store.reset();
        node.entry(&mut *store, &mut |store| {
            for node in ctx.nodes.iter().skip(i + 1) {
                if ctx.opts.sanity_check {
                    store.check()?;
                }
                trace!("  applying node {:?}", node);
                node.apply(store)?;
            }
            if ctx.opts.sanity_check {
                store.check()?;
            }
            Ok(())
        })?;
    }
    Ok(())
}

fn symlink_file(src: &Path, dst: &Path) -> io::Result<()> {
    let result = {
        #[cfg(target_family = "windows")]
        {
            std::os::windows::fs::symlink_file(src, dst)
        }
        #[cfg(target_family = "unix")]
        {
            std::os::unix::fs::symlink(src, dst)
        }
    };
    if result.is_err() {
        if let Ok(link_src) = fs::read_link(dst) {
            if link_src.canonicalize().ok() == src.canonicalize().ok() {
                //Link already exists
                trace!(
                    "  link \"{}\" <- \"{}\" already exists",
                    src.display(),
                    dst.display()
                );
                return Ok(());
            }
        }
    }
    result
}

fn symlink_dir(src: &Path, dst: &Path) -> io::Result<()> {
    let result = {
        #[cfg(target_family = "windows")]
        {
            std::os::windows::fs::symlink_dir(src, dst)
        }
        #[cfg(target_family = "unix")]
        {
            std::os::unix::fs::symlink(src, dst)
        }
    };
    if result.is_err() {
        if src.canonicalize().ok() == dst.canonicalize().ok() {
            //Paths are equivalent!
            debug!(
                "  link \"{}\" <- \"{}\" already exists (canonical paths are equivalent)",
                src.display(),
                dst.display()
            );
            return Ok(());
        }
        if src.canonicalize().ok() == fs::read_link(dst).and_then(|p| p.canonicalize()).ok() {
            //Link already exists
            debug!(
                "  link \"{}\" <- \"{}\" already exists",
                src.display(),
                dst.display()
            );
            return Ok(());
        }
    }
    result
}

fn load_cfg(path: &Path) -> Result<Opts> {
    //Replace all "\" for "\\", and all "\\" for "\", to allow for windows-style paths while still
    //allowing escapes for advanced users.
    let mut txt = fs::read_to_string(path)
        .with_context(|| anyhow!("failed to read config at \"{}\"", path.display()))?;
    let mut replacements = Vec::new();
    let mut skip_next_backslash = false;
    for (idx, _) in txt.match_indices('\\') {
        if skip_next_backslash {
            skip_next_backslash = false;
            continue;
        }
        if let Some(next_char) = txt.get(idx + 1..).and_then(|s| s.chars().next()) {
            if next_char == '\\' {
                //Convert double backslash to single backslash
                replacements.push((idx, ""));
                skip_next_backslash = true;
            } else {
                //Duplicate backslash
                replacements.push((idx, "\\\\"));
            }
        }
    }
    let mut added_bytes = 0;
    for (replace_idx, replace_by) in replacements {
        let replace_idx = (replace_idx as isize + added_bytes) as usize;
        txt.replace_range(replace_idx..replace_idx + 1, replace_by);
        added_bytes += replace_by.len() as isize - 1;
    }
    //Parse patched string
    ron::de::from_str(&txt)
        .with_context(|| anyhow!("failed to parse config at \"{}\"", path.display()))
}

fn save_cfg(path: &Path, opts: &Opts) -> Result<()> {
    ron::ser::to_writer_pretty(
        BufWriter::new(File::create(&path).with_context(|| anyhow!("failed to create file"))?),
        opts,
        default(),
    )
    .context("failed to serialize")?;
    Ok(())
}

struct BaseDirFinder<'a> {
    base_files: &'a [&'a str],
    threshold: f64,
    default_main_path: &'a str,
}
impl BaseDirFinder<'_> {
    /// Returns a `(base, main)` path tuple.
    fn find_base(&self, main_path: &Path, should_exist: bool) -> Result<(PathBuf, PathBuf)> {
        let mut base_path = main_path.to_path_buf();
        let mut cur_depth = 0;
        loop {
            //Check whether this path is the base path
            let score = self
                .base_files
                .iter()
                .map(|filename| base_path.join(filename).exists() as u8 as f64)
                .sum::<f64>()
                / self.base_files.len() as f64;
            if score >= self.threshold {
                //Base path!
                break;
            } else {
                //Keep looking
                if !base_path.pop() {
                    //Ran out of ancestors
                    bail!("could not find installation base");
                }
                cur_depth += 1;
            }
        }
        //Fix up main folder if depth is not correct
        let default_main_path: &Path = self.default_main_path.as_ref();
        let main_depth = default_main_path.iter().count();
        let mut tmp_main = main_path.to_path_buf();
        if cur_depth < main_depth {
            //Dig deeper
            tmp_main.extend(default_main_path.iter().skip(cur_depth));
            if should_exist && !tmp_main.is_dir() {
                //Undo the work, this folder does not exist
                tmp_main = main_path.to_path_buf();
            }
        } else if cur_depth > main_depth {
            //Go higher
            for _ in main_depth..cur_depth {
                tmp_main.pop();
            }
        }
        Ok((base_path, tmp_main))
    }
}

fn read_path_from_stdin() -> Result<String> {
    let mut path = String::new();
    io::stdin().read_line(&mut path).context("read stdin")?;
    let mut path = path.trim();
    if (path.starts_with('\'') && path.ends_with('\''))
        || (path.starts_with('"') && path.ends_with('"'))
    {
        path = path[1..path.len() - 1].trim();
    }
    Ok(path.to_string())
}

fn simfile_rng(sm: &Simfile, name: &str) -> FastRng {
    let seed = fxhash::hash64(&(&sm.music, &sm.title_trans, &sm.desc, name));
    FastRng::seed_from_u64(seed)
}

fn linear_map(in_min: f64, in_max: f64, out_min: f64, out_max: f64) -> impl Fn(f64) -> f64 {
    let m = (out_max - out_min) / (in_max - in_min);
    move |input| (input - in_min) * m + out_min
}

fn run() -> Result<()> {
    let load_cfg_from = std::env::args_os()
        .skip(1)
        .next()
        .map(|path| PathBuf::from(path));
    let opts = if let Some(cfg_path) = load_cfg_from {
        //Load from here
        let opts = load_cfg(&cfg_path)?;
        opts.apply();
        info!("loaded config from \"{}\"", cfg_path.display());
        opts
    } else {
        //Load/save config from default path
        let mut cfg_path: PathBuf = std::env::current_exe()
            .unwrap_or_default()
            .file_name()
            .unwrap_or_default()
            .into();
        cfg_path.set_extension("config.txt");
        match load_cfg(&cfg_path) {
            Ok(opts) => {
                opts.apply();
                info!("loaded config from \"{}\"", cfg_path.display());
                opts
            }
            Err(err) => {
                let opts = Opts::default();
                opts.apply();
                info!("failed to load config from default path: {:#}", err);
                if cfg_path.exists() {
                    info!("using default config");
                } else {
                    match save_cfg(&cfg_path, &opts) {
                        Ok(()) => {
                            info!("saved default config file");
                        }
                        Err(err) => {
                            warn!("failed to save default config: {:#}", err);
                        }
                    }
                }
                opts
            }
        }
    };
    let ctx = Ctx {
        sm_store: RefCell::new(default()),
        nodes: node::resolve_buckets(&opts.nodes).context("failed to resolve nodes")?,
        opts,
    };
    run_nodes(&ctx)?;
    Ok(())
}

fn main() {
    let start = Instant::now();
    match run() {
        Ok(()) => {
            info!(
                "finished in {}s",
                start.elapsed().as_millis() as f64 / 1000.
            );
        }
        Err(err) => {
            error!("fatal error: {:#}", err);
        }
    }
    eprintln!("hit enter to close this window");
    let _ = std::io::stdin().read_line(&mut String::new());
}
