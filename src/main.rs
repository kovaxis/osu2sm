use crate::prelude::*;

mod prelude {
    pub(crate) use crate::{
        osufile::{self, Beatmap, HitObject, TimingPoint},
        simfile::{Chart, Simfile},
        Ctx,
    };
    pub use anyhow::{anyhow, bail, ensure, Context, Error, Result};
    pub use fxhash::{FxHashMap as HashMap, FxHashSet as HashSet};
    pub use std::{
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
}

mod conv;
mod osufile;
mod simfile;

struct Ctx {
    base: PathBuf,
    out: PathBuf,
}

fn get_songs(ctx: &Ctx, mut on_bmset: impl FnMut(&Path, &[PathBuf]) -> Result<()>) -> Result<()> {
    let mut by_depth: Vec<Vec<PathBuf>> = Vec::new();
    for entry in WalkDir::new(&ctx.base).contents_first(true) {
        let entry = entry?;
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
    let bm = Beatmap::parse(bm_path).context("read/parse beatmap file")?;
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
            Err(err) => eprintln!("  error processing beatmap \"{}\": {:#}", bm_name, err),
        }
    }
    //Write output simfiles
    let rel = bmset_path
        .strip_prefix(&ctx.base)
        .context("find path relative to base")?;
    let out_base = ctx.out.join(rel);
    fs::create_dir_all(&out_base)
        .with_context(|| anyhow!("create output dir at \"{}\"", out_base.display()))?;
    for (i, sm) in out_sms.into_iter().enumerate() {
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
        println!("  writing simfile to \"{}\"", out_path.display());
        sm.save(&out_path)
            .with_context(|| anyhow!("write simfile to \"{}\"", out_path.display()))?;
    }
    Ok(())
}

fn run() -> Result<()> {
    let input_path: PathBuf = std::env::args_os()
        .skip(1)
        .next()
        .ok_or(anyhow!("expected input path"))?
        .into();
    let output_path = {
        let mut base_name = input_path.file_name().unwrap_or_default().to_os_string();
        base_name.push("Out");
        let mut out_path = input_path.clone();
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
        out_path
    };
    let ctx = Ctx {
        base: input_path,
        out: output_path,
    };
    println!("scanning for beatmaps in \"{}\"", ctx.base.display());
    println!("outputting simfiles in \"{}\"", ctx.out.display());
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
