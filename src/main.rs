use crate::prelude::*;

mod prelude {
    pub(crate) use crate::{
        osufile::{self, Beatmap, TimingPoint},
        simfile::{BeatPos, Chart, Difficulty, Gamemode, Note, Simfile},
        Ctx,
    };
    pub use anyhow::{anyhow, bail, ensure, Context, Error, Result};
    pub use fxhash::{FxHashMap as HashMap, FxHashSet as HashSet};
    pub use serde::{Deserialize, Serialize};
    pub use std::{
        cmp,
        convert::{TryFrom, TryInto},
        ffi::{OsStr, OsString},
        fmt::{self, Write as _},
        fs::{self, File},
        io::{self, BufRead, BufReader, BufWriter, Read, Write},
        mem, ops,
        path::{Path, PathBuf},
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
mod osufile;
mod simfile;

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Hash)]
enum CopyMethod {
    Hardlink,
    Symlink,
    Copy,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Opts {
    /// The input folder to scan.
    input: PathBuf,
    /// The output folder for converted simfiles.
    output: PathBuf,
    /// Whether to output unicode names or use ASCII only.
    unicode: bool,
    /// How to copy over audio and image files.
    copy: Vec<CopyMethod>,
    /// Whether to ignore incompatible-mode errors (there are too many and they are not terribly
    /// useful).
    ignore_mode_errors: bool,
    /// How much offset to apply to osu! HitObject and TimingPoint times when converting them
    /// to StepMania simfiles, in milliseconds.
    offset: f64,
}
impl Default for Opts {
    fn default() -> Opts {
        Opts {
            input: "".into(),
            output: "".into(),
            unicode: false,
            copy: vec![CopyMethod::Hardlink, CopyMethod::Symlink, CopyMethod::Copy],
            ignore_mode_errors: false,
            offset: 0.,
        }
    }
}

struct Ctx {
    opts: Opts,
}

fn get_songs(ctx: &Ctx, mut on_bmset: impl FnMut(&Path, &[PathBuf]) -> Result<()>) -> Result<()> {
    let mut by_depth: Vec<Vec<PathBuf>> = Vec::new();
    for entry in WalkDir::new(&ctx.opts.input).contents_first(true) {
        let entry = entry.context("failed to scan input directory")?;
        let depth = entry.depth();
        if depth < by_depth.len() {
            //Close directories
            for dir in by_depth.drain(depth..) {
                if !dir.is_empty() {
                    if let Err(e) = on_bmset(entry.path(), &dir[..]) {
                        eprintln!(
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
                    eprintln!("do not run on a .osu file, run on the beatmapset folder instead");
                }
            }
        }
    }
    Ok(())
}

fn process_beatmap(ctx: &Ctx, bmset_path: &Path, bm_path: &Path) -> Result<Simfile> {
    let bm = Beatmap::parse(ctx, bm_path).context("read/parse beatmap file")?;
    let sm = conv::convert(ctx, bmset_path, bm_path, bm)?;
    Ok(sm)
}

fn process_beatmapset(ctx: &Ctx, bmset_path: &Path, bm_paths: &[PathBuf]) -> Result<()> {
    println!("processing \"{}\":", bmset_path.display());
    //Parse and convert beatmaps
    let mut out_sms: Vec<Simfile> = Vec::new();
    for bm_path in bm_paths {
        let result = process_beatmap(ctx, bmset_path, bm_path);
        let bm_name = bm_path.file_name().unwrap_or_default().to_string_lossy();
        match result {
            Ok(mut sm) => {
                println!("  processed beatmap \"{}\" successfully", bm_name);
                let mut merged = false;
                for out_sm in out_sms.iter_mut() {
                    if (out_sm.title == sm.title || out_sm.title_trans == sm.title_trans)
                        && (out_sm.artist == sm.artist || out_sm.artist_trans == sm.artist_trans)
                        && out_sm.music == sm.music
                    {
                        //These two simfiles can be merged
                        out_sm.charts.append(&mut sm.charts);
                        merged = true;
                        break;
                    }
                }
                if !merged {
                    out_sms.push(sm);
                }
            }
            Err(err) => {
                if !ctx.opts.ignore_mode_errors || !err.to_string().contains("mode not supported") {
                    eprintln!("  error processing beatmap \"{}\": {:#}", bm_name, err);
                }
            }
        }
    }
    //Write output simfiles
    if !out_sms.is_empty() {
        //Resolve and create base output folder
        let rel = bmset_path
            .strip_prefix(&ctx.opts.input)
            .context("find path relative to base")?;
        let out_base = ctx.opts.output.join(rel);
        fs::create_dir_all(&out_base)
            .with_context(|| anyhow!("create output dir at \"{}\"", out_base.display()))?;
        //Do not copy files twice
        let mut already_copied: HashSet<PathBuf> = HashSet::default();
        for (i, mut sm) in out_sms.into_iter().enumerate() {
            //Solve difficulty conflicts
            sm.spread_difficulties()?;
            //Decide the output filename
            let mut title = sm.title_trans.clone();
            if title.is_empty() {
                title = sm.title.clone();
            }
            let mut filename = title
                .chars()
                .map(|c| if c as u32 >= 128 { '_' } else { c })
                .collect::<String>();
            if i > 0 {
                write!(filename, "{}", i).unwrap();
            }
            let mut out_path: PathBuf = out_base.join(&filename);
            out_path.set_extension("sm");
            //Write simfile
            println!("  writing simfile to \"{}\"", out_path.display());
            sm.save(&out_path)
                .with_context(|| anyhow!("write simfile to \"{}\"", out_path.display()))?;
            //Copy over dependencies (backgrounds, audio, etc...)
            for dep_name in sm.file_deps() {
                if !already_copied.contains(dep_name) {
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
                    println!("  copying file dependency \"{}\"", dep_name.display());
                    let dep_src = bmset_path.join(dep_name);
                    let dep_dst = out_base.join(dep_name);
                    copy_with_method(ctx, &dep_src, &dep_dst)?;
                }
            }
        }
    }
    Ok(())
}

fn copy_with_method(ctx: &Ctx, src: &Path, dst: &Path) -> Result<()> {
    let mut errors: Vec<Error> = Vec::new();
    macro_rules! method {
        ($($code:tt)*) => {{
            match {$($code)*} {
                Ok(()) => {
                    return Ok(());
                }
                Err(err) => {
                    errors.push(err);
                }
            }
        }};
    }
    for method in ctx.opts.copy.iter() {
        match method {
            CopyMethod::Copy => method! {
                fs::copy(src, dst).context("failed to do standard copy")?;
                Ok(())
            },
            CopyMethod::Hardlink => method! {
                fs::hard_link(src, dst).context("failed to create hardlink")?;
                Ok(())
            },
            CopyMethod::Symlink => method! {
                #[allow(deprecated)]
                {
                    fs::soft_link(src, dst).context("failed to create symlink")?;
                }
                Ok(())
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

fn load_cfg(path: &Path) -> Result<Opts> {
    ron::de::from_reader(
        File::open(&path)
            .with_context(|| anyhow!("failed to read config at \"{}\"", path.display()))?,
    )
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

fn run() -> Result<()> {
    let mut override_input = None;
    let mut load_cfg_from = None;
    if let Some(input_path) = std::env::args_os().skip(1).next() {
        //Load config/beatmaps from the given path
        let input_path: PathBuf = input_path.into();
        if input_path.is_dir() {
            override_input = Some(input_path);
        } else {
            load_cfg_from = Some(input_path);
        }
    }
    let mut opts = if let Some(cfg_path) = load_cfg_from {
        //Load from here
        let opts = load_cfg(&cfg_path)?;
        println!("loaded config from \"{}\"", cfg_path.display());
        opts
    } else {
        //Load/save config from default path
        let mut cfg_path = std::env::current_exe().unwrap_or_default();
        cfg_path.set_extension("config.txt");
        match load_cfg(&cfg_path) {
            Ok(opts) => {
                println!("loaded config from \"{}\"", cfg_path.display());
                opts
            }
            Err(err) => {
                eprintln!(
                    "failed to load config from \"{}\": {:#}",
                    cfg_path.display(),
                    err
                );
                let opts = Opts::default();
                if cfg_path.exists() {
                    eprintln!("  using defaults");
                } else {
                    match save_cfg(&cfg_path, &opts) {
                        Ok(()) => {
                            eprintln!("  saved default config file to \"{}\"", cfg_path.display());
                        }
                        Err(err) => {
                            eprintln!(
                                "  failed to save default config to \"{}\": {:#}",
                                cfg_path.display(),
                                err
                            );
                        }
                    }
                }
                opts
            }
        }
    };
    if let Some(over) = override_input {
        opts.input = over;
    }
    if opts.output.as_os_str().is_empty() {
        let mut base_name = opts.input.file_name().unwrap_or_default().to_os_string();
        base_name.push("Out");
        let mut out_path = opts.input.clone();
        let mut i = 0;
        loop {
            let mut filename = base_name.clone();
            if i > 0 {
                filename.push(format!("{}", i));
            }
            i += 1;
            out_path.set_file_name(filename);
            if !out_path.exists() {
                break;
            }
        }
        opts.output = out_path;
    }
    let ctx = Ctx { opts };
    println!("scanning for beatmaps in \"{}\"", ctx.opts.input.display());
    println!("outputting simfiles in \"{}\"", ctx.opts.output.display());
    get_songs(&ctx, |bmset, bm_paths| {
        process_beatmapset(&ctx, bmset, bm_paths)
    })?;
    Ok(())
}

fn main() {
    match run() {
        Ok(()) => {}
        Err(err) => {
            eprintln!("fatal error: {:#}", err);
        }
    }
}
