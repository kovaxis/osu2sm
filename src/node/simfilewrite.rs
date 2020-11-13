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
    /// Attempt to create a symlink from the input root directory to the output `output` directory.
    /// This allows for faster conversion, in a way that simfiles are output in the same input
    /// directory.
    ///
    /// Note that on windows creating symlinks requires admin permissions!
    /// Once the symlink is created no special permissions are required though.
    pub in_place: bool,
    /// If the output directory is a symlink to somewhere, this is it.
    /// Cannot be set from the config, it is only used as an internal cache.
    #[serde(skip)]
    pub in_place_from: RefCell<Option<PathBuf>>,
    /// Remove all files in the output directory or subdirectories matching the `osu2sm-*.sm`
    /// filename, where `*` stands for anything.
    pub cleanup: bool,
}

impl Default for SimfileWrite {
    fn default() -> Self {
        Self {
            from: default(),
            output: "".into(),
            fix_output: true,
            in_place: true,
            in_place_from: RefCell::new(None),
            copy: {
                //Leave symlinks for last on windows, since they require admin permissions
                #[cfg(target_family = "windows")]
                {
                    vec![
                        CopyMethod::Hardlink,
                        CopyMethod::Copy,
                        CopyMethod::Symlink,
                        CopyMethod::AssertIdentical,
                    ]
                }
                //On unix use them right off the bat, they're the best option
                #[cfg(target_family = "unix")]
                {
                    vec![
                        CopyMethod::Symlink,
                        CopyMethod::Hardlink,
                        CopyMethod::Copy,
                        CopyMethod::AssertIdentical,
                    ]
                }
            },
            cleanup: false,
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Hash)]
pub enum CopyMethod {
    /// Create a hardlink from source to destination.
    Hardlink,
    /// Create a symlink from source to destination (requires admin permissions on Windows).
    Symlink,
    /// Copy the file from source to destination.
    Copy,
    /// Only assert the source and destination files are identical.
    AssertIdentical,
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
        //Cleanup output
        if self.cleanup {
            info!(
                "cleanup enabled, removing all `osu2sm-*.sm` files under \"{}\"",
                self.output
            );
            let mut files_removed = 0;
            for file in WalkDir::new(&self.output) {
                let file = match file {
                    Ok(f) => f,
                    Err(err) => {
                        warn!("  failed to list files for cleanup: {:#}", err);
                        continue;
                    }
                };
                if file.file_type().is_file() {
                    let filename = file.file_name().to_string_lossy();
                    if filename.starts_with("osu2sm-") && filename.ends_with(".sm") {
                        match fs::remove_file(file.path()) {
                            Ok(()) => {
                                files_removed += 1;
                            }
                            Err(err) => {
                                warn!(
                                    "  failed to remove file \"{}\" while cleaning up: {:#}",
                                    file.path().display(),
                                    err
                                );
                            }
                        }
                    }
                }
            }
            info!("  removed {} files", files_removed);
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
        //Handle in-place-ness lazily on the first simfile
        if self.in_place {
            let mut in_place_from = self.in_place_from.borrow_mut();
            let in_place_from = in_place_from.get_or_insert_with(|| {
                //Attempt to create symlink for in-place conversion
                match symlink_dir(root_path.as_ref(), self.output.as_ref())
                    .context("failed to create output symlink pointing to input")
                {
                    Ok(()) => {
                        info!("  enabled in-place conversion");
                        root_path.clone().into()
                    }
                    Err(err) => {
                        warn!("  disabled in-place conversion: {:#}", err);
                        PathBuf::new()
                    }
                }
            });
            if !in_place_from.as_os_str().is_empty() {
                //The symlink points to `in_place_from`
                //Make sure the input simfile has this same root
                ensure!(
                    root_path == in_place_from.as_os_str(),
                    "can only convert simfiles in-place from \"{}\", but received a simfile with root \"{}\"",
                    in_place_from.display(),
                    root_path,
                );
            }
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

fn in_place_enabled(conf: &SimfileWrite) -> bool {
    conf.in_place_from
        .borrow()
        .as_ref()
        .map(|path| !path.as_os_str().is_empty())
        .unwrap_or(false)
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
    let out_base = if in_place_enabled(conf) {
        set_path.to_path_buf()
    } else {
        let rel = set_path
            .strip_prefix(root_path)
            .context("find path relative to base")?;
        Path::new(&conf.output).join(rel)
    };
    //Create base output folder
    if !in_place_enabled(conf) {
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
    if !in_place_enabled(conf) {
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
                match copy_with_methods(&conf.copy, &dep_src, &dep_dst) {
                    Ok(method) => {
                        info!(
                            "  copied dependency \"{}\" using {:?}",
                            dep_name.display(),
                            method
                        );
                    }
                    Err(err) => {
                        error!(
                            "  failed to copy dependency \"{}\": {:#}",
                            dep_name.display(),
                            err
                        );
                    }
                }
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
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent).context("create parent directory")?;
    }
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
            CopyMethod::AssertIdentical => method! {method,
                assert_identical(src, dst).context("source and destination are not identical")
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

fn assert_identical(src: &Path, dst: &Path) -> Result<()> {
    let mut src = File::open(src).context("failed to open source file")?;
    let mut dst = File::open(dst).context("failed to open destination file")?;
    if let (Ok(src_meta), Ok(dst_meta)) = (src.metadata(), dst.metadata()) {
        ensure!(
            src_meta.len() == dst_meta.len(),
            "files are not the same size ({} != {})",
            src_meta.len(),
            dst_meta.len()
        );
    }
    let buf_cap = 16 * 1024;
    let mut buf = vec![0; buf_cap * 2];
    let (buf_src, buf_dst) = buf.split_at_mut(buf_cap);
    loop {
        let len = src.read(buf_src)?;
        if len == 0 {
            ensure!(
                dst.read(buf_dst)? == 0,
                "source reached eof before destination"
            );
            break;
        }
        let mut read_dst = 0;
        while read_dst < len {
            let read_bytes = dst.read(&mut buf_dst[read_dst..len])?;
            ensure!(read_bytes > 0, "destination reached eof before source");
            read_dst += read_bytes;
        }
        ensure!(
            buf_src[..len] == buf_dst[..len],
            "files do not have the same contents"
        );
    }
    Ok(())
}
