use crate::prelude::*;

mod prelude {
    pub(crate) use crate::{
        linear_map,
        osufile::{self, Beatmap, TimingPoint},
        simfile::{BeatPos, ControlPoint, Difficulty, Gamemode, Note, Simfile, ToTime},
        simfile_rng,
        transform::{ConcreteTransform, SimfileStore, Transform},
        Ctx,
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

mod conv;
pub mod osufile;
pub mod simfile;
pub mod transform;

const OSU_AUTODETECT: BaseDirFinder = BaseDirFinder {
    base_files: &[
        "collection.db",
        "osu!.db",
        "presence.db",
        "scores.db",
        "Replays",
        "Skins",
    ],
    threshold: 4.9 / 6.,
    default_main_path: "Songs",
};
const STEPMANIA_AUTODETECT: BaseDirFinder = BaseDirFinder {
    base_files: &[
        "Announcers",
        "BackgroundEffects",
        "BackgroundTransitions",
        "BGAnimations",
        "Characters",
        "Courses",
        "Data",
        "Docs",
        "NoteSkins",
        "Scripts",
        "Themes",
    ],
    threshold: 0.8,
    default_main_path: "Songs/Osu",
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
struct Opts {
    /// The input folder to scan.
    input: PathBuf,
    /// Whether to automatically fix input path using osu! installation folder autodetect.
    fix_input: bool,
    /// The osu! base installation folder.
    /// Will be autodetected if left empty.
    osu_dir: Option<PathBuf>,
    /// The output folder for converted simfiles.
    output: PathBuf,
    /// Whether to automatically fix output path using stepmania installation folder autodetect.
    fix_output: bool,
    /// The stepmania base installation folder.
    /// Will be autodetected if left empty.
    stepmania_dir: Option<PathBuf>,
    /// What gamemodes (keycounts) to convert compatible-keycount maps into.
    gamemodes: Vec<Gamemode>,
    /// Transformations to apply to converted simfiles before saving.
    transforms: Vec<ConcreteTransform>,
    /// Whether to output unicode names or use ASCII only.
    unicode: bool,
    /// Whether to create a simple directory link to the input, and create the `.sm` files in-place.
    in_place: bool,
    /// Whether to output videos or just use the static background.
    video: bool,
    /// How to copy over audio and image files.
    copy: Vec<CopyMethod>,
    /// Whether to ignore incompatible-mode errors (there are too many and they are not terribly
    /// useful).
    ignore_mode_errors: bool,
    /// How much offset to apply to osu! HitObject and TimingPoint times when converting them
    /// to StepMania simfiles, in milliseconds.
    offset: f64,
    /// A logspec string (see
    // https://https://docs.rs/flexi_logger/0.16.1/flexi_logger/struct.LogSpecification.html).
    log: String,
    /// Whether to log to a file.
    log_file: bool,
    /// Enable logging to stderr.
    log_stderr: bool,
    /// Enable logging to stdout.
    log_stdout: bool,
    /// Do a random trim of input beatmaps, for debug purposes.
    /// If `Some`, uses the given chance of allowing a beatmap and the given random seed.
    debug: Option<DebugOpts>,
    /// Query the length of the audio files in order to create an accurate preview range.
    /// However, querying the length of an audio file can take quite some time, so disable for
    /// speed.
    query_audio_len: bool,
    /// Do a cleanup run before processing beatmaps.
    /// Removes all `.sm` files from directories with `.osu` files.
    cleanup: bool,
}
impl Default for Opts {
    fn default() -> Opts {
        Opts {
            input: "".into(),
            fix_input: true,
            osu_dir: None,
            output: "".into(),
            fix_output: true,
            stepmania_dir: None,
            gamemodes: {
                use crate::simfile::Gamemode::*;
                // Supported: 3K - 10K
                vec![
                    DanceThreepanel,
                    DanceSingle,
                    DanceSolo,
                    DanceDouble,
                    PumpSingle,
                    PumpHalfdouble,
                    PumpDouble,
                    Kb7Single,
                    PnmFive,
                    PnmNine,
                ]
            },
            transforms: vec![
                transform::Remap {
                    gamemode: Gamemode::DanceSingle,
                    ..default()
                }
                .into(),
                transform::Filter {
                    ops: vec![(
                        transform::Property::Gamemode,
                        transform::FilterOp::Allow(vec!["DanceSingle".to_string()]),
                    )],
                    ..default()
                }
                .into(),
                transform::SimfileFix { ..default() }.into(),
            ],
            unicode: false,
            in_place: true,
            video: true,
            copy: vec![CopyMethod::Hardlink, CopyMethod::Symlink, CopyMethod::Copy],
            ignore_mode_errors: true,
            offset: 0.,
            log: "info".to_string(),
            log_file: true,
            log_stderr: true,
            log_stdout: false,
            debug: None,
            query_audio_len: true,
            cleanup: false,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DebugOpts {
    allow_chance: f64,
    allow_seed: u64,
    /// Entries must be lowercase.
    blacklist: Vec<String>,
    /// Entries must be lowercase.
    whitelist: Vec<String>,
}
impl Default for DebugOpts {
    fn default() -> Self {
        Self {
            allow_chance: 1.,
            allow_seed: 0,
            blacklist: vec![],
            whitelist: vec![],
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Hash)]
enum CopyMethod {
    Hardlink,
    Symlink,
    Copy,
}

struct Ctx {
    opts: Opts,
    sm_store: RefCell<SimfileStore>,
    transforms: Vec<Box<dyn Transform>>,
}

fn get_songs(ctx: &Ctx, mut on_bmset: impl FnMut(&Path, &[PathBuf]) -> Result<()>) -> Result<()> {
    let mut by_depth: Vec<Vec<PathBuf>> = Vec::new();
    let mut randtrim = ctx
        .opts
        .debug
        .as_ref()
        .filter(|dbg| dbg.allow_chance < 1.)
        .map(|dbg| (dbg.allow_chance, FastRng::seed_from_u64(dbg.allow_seed)));
    for entry in WalkDir::new(&ctx.opts.input).contents_first(true) {
        let entry = entry.context("failed to scan input directory")?;
        let depth = entry.depth();
        if depth < by_depth.len() {
            //Close directories
            for dir in by_depth.drain(depth..) {
                if let Some((chance, rng)) = &mut randtrim {
                    if !rng.gen_bool(*chance) {
                        continue;
                    }
                }
                if let Some(dbg) = ctx.opts.debug.as_ref() {
                    let path = entry
                        .path()
                        .strip_prefix(&ctx.opts.input)
                        .ok()
                        .and_then(Path::to_str)
                        .unwrap_or_default()
                        .to_lowercase();
                    if dbg.blacklist.iter().any(|black| path.contains(black)) {
                        //Path contains blacklisted keywords
                        continue;
                    }
                    if !dbg.whitelist.is_empty()
                        && !dbg.whitelist.iter().any(|white| path.contains(white))
                    {
                        //Path is not whitelisted
                        continue;
                    }
                }
                if !dir.is_empty() {
                    if let Err(e) = on_bmset(entry.path(), &dir[..]) {
                        warn!(
                            "  error processing beatmapset at \"{}\": {:#}",
                            entry.path().display(),
                            e
                        );
                    }
                }
            }
        } else {
            //Add new by_depth entries
            while depth > by_depth.len() {
                by_depth.push(Vec::new());
            }
        }
        if entry.file_type().is_file() {
            if entry.path().extension() == Some("osu".as_ref()) {
                let bm_path = entry.into_path();
                if depth > 0 {
                    by_depth[depth - 1].push(bm_path);
                } else {
                    warn!("do not run on a .osu file, run on the beatmapset folder instead");
                }
            }
        }
    }
    Ok(())
}

fn process_beatmap<'a>(
    ctx: &'a Ctx,
    bmset_path: &Path,
    bm_path: &Path,
    out: impl FnMut(Box<Simfile>),
) -> Result<()> {
    let bm = Beatmap::parse(ctx.opts.offset, bm_path).context("read/parse beatmap file")?;
    conv::convert(ctx, bmset_path, bm_path, bm, out)?;
    Ok(())
}

fn process_beatmapset(ctx: &Ctx, bmset_path: &Path, bm_paths: &[PathBuf]) -> Result<()> {
    info!("processing \"{}\":", bmset_path.display());
    //Parse and convert beatmaps
    let mut flat_simfiles = Vec::new();
    for bm_path in bm_paths {
        let old_simfile_count = flat_simfiles.len();
        let result = process_beatmap(ctx, bmset_path, bm_path, |sm| flat_simfiles.push(sm));
        let bm_name = bm_path.file_name().unwrap_or_default().to_string_lossy();
        match result {
            Ok(()) => {
                debug!(
                    "  loaded beatmap \"{}\" successfully into {} simfiles",
                    bm_name,
                    flat_simfiles.len() as isize - old_simfile_count as isize,
                );
            }
            Err(err) => {
                if !ctx.opts.ignore_mode_errors || !err.to_string().contains("mode not supported") {
                    error!("  error processing beatmap \"{}\": {:#}", bm_name, err);
                }
            }
        }
    }
    //Transform simfiles
    let mut sm_store = ctx.sm_store.borrow_mut();
    sm_store.reset(flat_simfiles);
    for trans in ctx.transforms.iter() {
        trace!("  applying transform {:?}", trans);
        match trans.apply(&mut sm_store) {
            Ok(()) => {}
            Err(err) => {
                error!("  transform {:?} failed to apply: {:#}", trans, err);
            }
        }
    }
    //Organize output simfiles
    let mut by_music: HashMap<PathBuf, Vec<Box<Simfile>>> = HashMap::default();
    for sm in sm_store.take_output()? {
        let list = by_music
            .entry(
                AsRef::<Path>::as_ref(sm.music.as_ref().map(|p| p.as_os_str()).unwrap_or_default())
                    .to_path_buf(),
            )
            .or_default();
        list.push(sm);
    }
    //Resolve output folder
    let out_base = if ctx.opts.in_place {
        bmset_path.to_path_buf()
    } else {
        let rel = bmset_path
            .strip_prefix(&ctx.opts.input)
            .context("find path relative to base")?;
        ctx.opts.output.join(rel)
    };
    //Cleanup output
    if ctx.opts.cleanup {
        for file in WalkDir::new(&out_base).min_depth(1).max_depth(1) {
            let file = match file {
                Ok(f) => f,
                Err(err) => {
                    warn!("  failed to list beatmapset files for cleanup: {:#}", err);
                    break;
                }
            };
            let filename: &Path = file.file_name().as_ref();
            if filename.extension() == Some("sm".as_ref()) {
                if let Err(err) = fs::remove_file(file.path()) {
                    warn!(
                        "  failed to remove file \"{}\" while cleaning up: {:#}",
                        file.path().display(),
                        err
                    );
                }
            }
        }
    }
    //Write output simfiles
    if !by_music.is_empty() {
        //Create base output folder
        if !ctx.opts.in_place {
            fs::create_dir_all(&out_base)
                .with_context(|| anyhow!("create output dir at \"{}\"", out_base.display()))?;
        }
        //Do not copy files twice
        let mut already_copied: HashSet<PathBuf> = HashSet::default();
        for (audio_name, sms) in by_music.into_iter() {
            //Decide the output filename
            let filename = format!(
                "osu2sm-{}.sm",
                audio_name.file_stem().unwrap_or_default().to_string_lossy()
            );
            let out_path: PathBuf = out_base.join(&filename);
            //Write simfile
            debug!("  writing simfile to \"{}\"", out_path.display());
            Simfile::save(&out_path, sms.iter().map(|sm| &**sm))
                .with_context(|| anyhow!("write simfile to \"{}\"", out_path.display()))?;
            //Copy over dependencies (backgrounds, audio, etc...)
            if !ctx.opts.in_place {
                for sm in sms.iter() {
                    for dep_name in sm.file_deps() {
                        if already_copied.contains(dep_name) {
                            continue;
                        }
                        already_copied.insert(dep_name.to_path_buf());
                        //Make sure no rogue '..' or 'C:\System32' appear
                        for comp in dep_name.components() {
                            use std::path::Component;
                            match comp {
                                Component::Normal(_) | Component::CurDir => {}
                                _ => bail!("invalid simfile dependency \"{}\"", dep_name.display()),
                            }
                        }
                        //Copy the dependency over to the destination folder
                        let dep_src = bmset_path.join(dep_name);
                        let dep_dst = out_base.join(dep_name);
                        let method = copy_with_method(ctx, &dep_src, &dep_dst)?;
                        info!(
                            "  copied dependency \"{}\" using {:?}",
                            dep_name.display(),
                            method
                        );
                    }
                }
            }
        }
    }
    Ok(())
}

fn copy_with_method(ctx: &Ctx, src: &Path, dst: &Path) -> Result<CopyMethod> {
    debug!("  copying \"{}\" to \"{}\"", src.display(), dst.display());
    let mut errors: Vec<Error> = Vec::new();
    macro_rules! method {
        ($method:expr, $($code:tt)*) => {{
            match {$($code)*} {
                Ok(()) => {
                    return Ok($method);
                }
                Err(err) => {
                    debug!("    method {:?} failed: {:#}", $method, err);
                    errors.push(err);
                }
            }
        }};
    }
    for &method in ctx.opts.copy.iter() {
        match method {
            CopyMethod::Copy => method! {method,
                fs::copy(src, dst).context("failed to do standard copy").map(|_| ())
            },
            CopyMethod::Hardlink => method! {method,
                fs::hard_link(src, dst).context("failed to create hardlink")
            },
            CopyMethod::Symlink => method! {method,
                symlink_file(src, dst).context("failed to create symlink")
            },
        }
    }
    //Ran out of methods
    let mut errstr = format!(
        "could not copy file from \"{}\" to \"{}\":",
        src.display(),
        dst.display()
    );
    for err in errors {
        write!(errstr, "\n  {:#}", err).unwrap();
    }
    bail!(errstr)
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
        if let Ok(link_src) = fs::read_link(dst) {
            if link_src.canonicalize().ok() == src.canonicalize().ok() {
                //Link already exists
                debug!(
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

fn read_path_from_stdin() -> Result<PathBuf> {
    let mut path = String::new();
    io::stdin().read_line(&mut path).context("read stdin")?;
    let mut path = path.trim();
    if (path.starts_with('\'') && path.ends_with('\''))
        || (path.starts_with('"') && path.ends_with('"'))
    {
        path = path[1..path.len() - 1].trim();
    }
    Ok(path.into())
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
    let mut opts = if let Some(cfg_path) = load_cfg_from {
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
                warn!("failed to load config from default path: {:#}", err);
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
    if opts.input.as_os_str().is_empty() {
        eprintln!();
        eprintln!("drag and drop your osu! song folder into this window, then press enter");
        opts.input = read_path_from_stdin()?;
    }
    if opts.fix_input || opts.osu_dir.is_none() {
        debug!("autodetecting osu! installation");
        match OSU_AUTODETECT.find_base(&opts.input, true) {
            Ok((base, main)) => {
                debug!(
                    "  determined osu! to be installed at \"{}\"",
                    base.display()
                );
                debug!("  songs dir at \"{}\"", main.display());
                opts.osu_dir.get_or_insert(base);
                if opts.fix_input && opts.input != main {
                    info!(
                        "fixed input path: \"{}\" -> \"{}\"",
                        opts.input.display(),
                        main.display()
                    );
                    opts.input = main;
                }
            }
            Err(err) => {
                warn!("could not find osu! install dir: {:#}", err);
            }
        }
    }
    if opts.output.as_os_str().is_empty() {
        eprintln!();
        eprintln!("drag and drop your stepmania song folder into this window, then press enter");
        opts.output = read_path_from_stdin()?;
    }
    if opts.fix_output || opts.stepmania_dir.is_none() {
        debug!("autodetecting stepmania installation");
        match STEPMANIA_AUTODETECT.find_base(&opts.output, false) {
            Ok((base, main)) => {
                debug!(
                    "  determined stepmania to be installed at \"{}\"",
                    base.display()
                );
                debug!("  songs dir at \"{}\"", main.display());
                opts.stepmania_dir.get_or_insert(base);
                if opts.fix_output && opts.output != main {
                    info!(
                        "fixed output path: \"{}\" -> \"{}\"",
                        opts.output.display(),
                        main.display()
                    );
                    opts.output = main;
                }
            }
            Err(err) => {
                warn!("could not find stepmania install dir: {:#}", err);
            }
        }
    }
    let mut ctx = Ctx {
        sm_store: RefCell::new(default()),
        transforms: transform::resolve_buckets(&opts.transforms)
            .context("failed to resolve transforms")?,
        opts,
    };
    info!("scanning for beatmaps in \"{}\"", ctx.opts.input.display());
    info!("outputting simfiles in \"{}\"", ctx.opts.output.display());
    if ctx.opts.in_place {
        match symlink_dir(&ctx.opts.input, &ctx.opts.output)
            .context("failed to create output symlink pointing to input")
        {
            Ok(()) => {
                info!("  enabled in-place conversion");
            }
            Err(err) => {
                ctx.opts.in_place = false;
                warn!("  disabled in-place conversion: {:#}", err);
            }
        }
    }
    if ctx.opts.cleanup {
        info!("cleanup mode enabled, removing stray `.sm` files");
    }
    get_songs(&ctx, |bmset, bm_paths| {
        process_beatmapset(&ctx, bmset, bm_paths)
    })?;
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
