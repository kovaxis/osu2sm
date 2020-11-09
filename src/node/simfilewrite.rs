//! Takes a bunch of simfiles as input and writes them out to the filesystem.

use crate::node::prelude::*;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct SimfileWrite {
    pub from: BucketId,
    /// The path to the output directory (a StepMania song group).
    pub output: String,
    /// Whether to automatically correct output paths if they point somewhere within a StepMania
    /// installation.
    pub fix_output: bool,
    /// Which methods to try for copying "dependency" files, such as `.mp3` and `.jpg` files.
    pub copy: Vec<CopyMethod>,
    /// Attempt to create a directory symlink to this input directory, speeding up the process.
    pub in_place_from: Option<String>,
    /// Remove any leftover `.sm` files.
    pub cleanup: bool,
}

impl Default for SimfileWrite {
    fn default() -> Self {
        Self {
            from: default(),
            output: "".into(),
            fix_output: true,
            in_place_from: None,
            cleanup: false,
            copy: vec![CopyMethod::Hardlink, CopyMethod::Symlink, CopyMethod::Copy],
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Hash)]
pub enum CopyMethod {
    Hardlink,
    Symlink,
    Copy,
}

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

impl Node for SimfileWrite {
    fn prepare(&mut self) -> Result<()> {
        //Fetch simfile output if empty
        if self.output.is_empty() {
            eprintln!();
            eprintln!(
                "drag and drop your stepmania song folder into this window, then press enter"
            );
            self.output = crate::read_path_from_stdin()?;
        }
        if self.fix_output {
            debug!("autodetecting stepmania installation");
            match STEPMANIA_AUTODETECT.find_base(self.output.as_ref(), false) {
                Ok((base, main)) => {
                    let main = main.into_os_string().into_string().map_err(|main| {
                        anyhow!(
                            "invalid non-utf8 fixed output path \"{}\"",
                            main.to_string_lossy()
                        )
                    })?;
                    debug!(
                        "  determined stepmania to be installed at \"{}\"",
                        base.display()
                    );
                    debug!("  songs dir at \"{}\"", main);
                    if self.output != main {
                        info!("fixed output path: \"{}\" -> \"{}\"", self.output, main);
                        self.output = main;
                    }
                }
                Err(err) => {
                    warn!("could not find stepmania install dir: {:#}", err);
                }
            }
        }
        if let Some(in_place_from) = self.in_place_from.as_deref() {
            //Attempt to create symlink for in-place conversion
            match symlink_dir(in_place_from.as_ref(), self.output.as_ref())
                .context("failed to create output symlink pointing to input")
            {
                Ok(()) => {
                    info!("  enabled in-place conversion");
                }
                Err(err) => {
                    self.in_place_from = None;
                    warn!("  disabled in-place conversion: {:#}", err);
                }
            }
        }
        if self.cleanup {
            info!("cleanup mode enabled, removing stray `.sm` files");
        }
        info!("outputting simfiles in \"{}\"", self.output);
        Ok(())
    }
    fn apply(&self, store: &mut SimfileStore) -> Result<()> {
        //Organize output simfiles
        let mut by_music: HashMap<PathBuf, Vec<Box<Simfile>>> = HashMap::default();
        store.get_each(&self.from, |_, mut sm| {
            //Fix some `.sm` quirks
            sm.fix_tails()?;
            //Append to the appropiate list
            let list = by_music
                .entry(
                    AsRef::<Path>::as_ref(
                        sm.music.as_ref().map(|p| p.as_os_str()).unwrap_or_default(),
                    )
                    .to_path_buf(),
                )
                .or_default();
            list.push(sm);
            Ok(())
        })?;
        //Get globals
        let root_path = store.global_get_expect("root")?;
        let set_path = store.global_get_expect("base")?;
        //Ensure in-place-ness is correct
        if let Some(in_place_from) = self.in_place_from.as_deref() {
            ensure!(
                root_path == in_place_from,
                "can only convert simfiles in-place from \"{}\", but received a simfile with root \"{}\"", in_place_from, root_path,
            );
        }
        //Write output simfiles
        for (_music_path, simfiles) in by_music {
            //Write a single `.sm` for these simfiles
            write_sm(self, root_path.as_ref(), set_path.as_ref(), &simfiles)?;
        }
        Ok(())
    }
    fn buckets_mut<'a>(&'a mut self) -> BucketIter<'a> {
        Box::new(iter::once((BucketKind::Input, &mut self.from)))
    }
}

fn write_sm(
    conf: &SimfileWrite,
    root_path: &Path,
    set_path: &Path,
    sms: &[Box<Simfile>],
) -> Result<()> {
    if sms.is_empty() {
        //Skip empty beatmapsets
        return Ok(());
    }
    //Resolve output folder
    let out_base = if conf.in_place_from.is_some() {
        set_path.to_path_buf()
    } else {
        let rel = set_path
            .strip_prefix(root_path)
            .context("find path relative to base")?;
        Path::new(&conf.output).join(rel)
    };
    //Cleanup output
    if conf.cleanup {
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
    //Create base output folder
    if conf.in_place_from.is_none() {
        fs::create_dir_all(&out_base)
            .with_context(|| anyhow!("create output dir at \"{}\"", out_base.display()))?;
    }
    //Do not copy files twice
    let mut already_copied: HashSet<PathBuf> = HashSet::default();
    //Decide the output filename
    let filename = format!(
        "osu2sm-{}.sm",
        sms[0]
            .music
            .as_ref()
            .map(|m| m.file_stem().unwrap_or_default().to_string_lossy())
            .unwrap_or_default()
    );
    let out_path: PathBuf = out_base.join(&filename);
    //Write simfile
    debug!("  writing simfile to \"{}\"", out_path.display());
    Simfile::save(&out_path, sms.iter().map(|sm| &**sm))
        .with_context(|| anyhow!("write simfile to \"{}\"", out_path.display()))?;
    //Copy over dependencies (backgrounds, audio, etc...)
    if conf.in_place_from.is_none() {
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
                let dep_src = set_path.join(dep_name);
                let dep_dst = out_base.join(dep_name);
                let method = copy_with_methods(&conf.copy, &dep_src, &dep_dst)?;
                info!(
                    "  copied dependency \"{}\" using {:?}",
                    dep_name.display(),
                    method
                );
            }
        }
    }
    Ok(())
}

fn copy_with_methods<'a>(
    methods: &'a [CopyMethod],
    src: &Path,
    dst: &Path,
) -> Result<&'a CopyMethod> {
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
    for method in methods.iter() {
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
