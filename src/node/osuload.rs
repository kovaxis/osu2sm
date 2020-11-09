//! Take an osu! input directory and parse its beatmaps.

use crate::node::prelude::*;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct OsuLoad {
    pub into: BucketId,
    /// The input osu! song folder.
    pub input: String,
    /// Whether to attempt to automatically correct the path if it points to somewhere within an
    /// osu! installation.
    pub fix_input: bool,
    /// The offset to apply to osu! files, in milliseconds.
    pub offset: f64,
    /// Whether to read `.mp3` files to query audio length (for proper preview audio in the song
    /// wheel select).
    pub query_audio_len: bool,
    /// Which gamemodes to generate.
    pub gamemodes: Vec<Gamemode>,
    /// Whether to use the osu! unicode names or not.
    pub unicode: bool,
    /// Whether to use or ignore video files.
    pub video: bool,
    /// What is the chance to load a beatmapset.
    /// Defaults to `1` (of course).
    /// Intended for debug purposes.
    pub debug_allow_chance: f64,
    /// The random seed for `debug_allow_chance`, for reproducible results.
    pub debug_allow_seed: u64,
    /// Entries must be lowercase.
    pub blacklist: Vec<String>,
    /// Entries must be lowercase.
    pub whitelist: Vec<String>,
    /// Whether to ignore "incompatible mode" errors, which may be _too_ numerous.
    pub ignore_mode_errors: bool,
}

impl Default for OsuLoad {
    fn default() -> Self {
        Self {
            into: default(),
            input: "".into(),
            fix_input: true,
            offset: 0.,
            query_audio_len: true,
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
            unicode: false,
            video: true,
            debug_allow_chance: 1.,
            debug_allow_seed: 0,
            blacklist: vec![],
            whitelist: vec![],
            ignore_mode_errors: true,
        }
    }
}

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

impl Node for OsuLoad {
    fn prepare(&mut self) -> Result<()> {
        if self.input.is_empty() {
            eprintln!();
            eprintln!("drag and drop your osu! song folder into this window, then press enter");
            self.input = crate::read_path_from_stdin()?;
        }
        if self.fix_input {
            debug!("autodetecting osu! installation");
            match OSU_AUTODETECT.find_base(self.input.as_ref(), true) {
                Ok((base, main)) => {
                    let main = main.into_os_string().into_string().map_err(|main| {
                        anyhow!(
                            "invalid non-utf8 fixed input path \"{}\"",
                            main.to_string_lossy()
                        )
                    })?;
                    debug!(
                        "  determined osu! to be installed at \"{}\"",
                        base.display()
                    );
                    debug!("  songs dir at \"{}\"", main);
                    if self.input != main {
                        info!("fixed input path: \"{}\" -> \"{}\"", self.input, main);
                        self.input = main;
                    }
                }
                Err(err) => {
                    warn!("could not find osu! install dir: {:#}", err);
                }
            }
        }
        info!("scanning for beatmaps in \"{}\"", self.input);
        Ok(())
    }
    fn apply(&self, _store: &mut SimfileStore) -> Result<()> {
        Ok(())
    }
    fn buckets_mut(&mut self) -> BucketIter {
        Box::new(iter::once((BucketKind::Output, &mut self.into)))
    }
    fn entry(
        &self,
        store: &mut SimfileStore,
        on_bmset: &mut dyn FnMut(&mut SimfileStore) -> Result<()>,
    ) -> Result<()> {
        scan_folder(self, store, on_bmset)
    }
}

fn scan_folder(
    conf: &OsuLoad,
    store: &mut SimfileStore,
    on_bmset: &mut dyn FnMut(&mut SimfileStore) -> Result<()>,
) -> Result<()> {
    store.global_set("root", conf.input.to_string());
    let mut by_depth: Vec<Vec<PathBuf>> = Vec::new();
    let mut randtrim = if conf.debug_allow_chance < 1. {
        Some(FastRng::seed_from_u64(conf.debug_allow_seed))
    } else {
        None
    };
    for entry in WalkDir::new(&conf.input).contents_first(true) {
        let entry = entry.context("failed to scan input directory")?;
        let depth = entry.depth();
        if depth < by_depth.len() {
            //Close directories
            for dir in by_depth.drain(depth..) {
                if let Some(rng) = &mut randtrim {
                    if !rng.gen_bool(conf.debug_allow_chance) {
                        continue;
                    }
                }
                if !conf.blacklist.is_empty() || !conf.whitelist.is_empty() {
                    let path = entry
                        .path()
                        .strip_prefix(&conf.input)
                        .ok()
                        .and_then(Path::to_str)
                        .unwrap_or_default()
                        .to_lowercase();
                    if conf.blacklist.iter().any(|black| path.contains(black)) {
                        //Path contains blacklisted keywords
                        continue;
                    }
                    if !conf.whitelist.is_empty()
                        && !conf.whitelist.iter().any(|white| path.contains(white))
                    {
                        //Path is not whitelisted
                        continue;
                    }
                }
                if !dir.is_empty() {
                    match process_beatmapset(conf, entry.path(), &dir[..]) {
                        Ok(simfiles) => {
                            store.global_set(
                                "base",
                                entry
                                    .path()
                                    .to_str()
                                    .ok_or(anyhow!(
                                        "non utf-8 beatmapset path \"{}\"",
                                        entry.path().display()
                                    ))?
                                    .to_string(),
                            );
                            store.put(&conf.into, simfiles);
                            on_bmset(store)?;
                        }
                        Err(e) => {
                            warn!(
                                "  error processing beatmapset at \"{}\": {:#}",
                                entry.path().display(),
                                e
                            );
                        }
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

fn process_beatmapset(
    conf: &OsuLoad,
    bmset_path: &Path,
    bm_paths: &[PathBuf],
) -> Result<Vec<Box<Simfile>>> {
    info!("processing \"{}\":", bmset_path.display());
    //Parse and convert beatmaps
    let mut flat_simfiles = Vec::new();
    for bm_path in bm_paths {
        let old_simfile_count = flat_simfiles.len();
        let result = process_beatmap(conf, bmset_path, bm_path, |sm| flat_simfiles.push(sm));
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
                if !conf.ignore_mode_errors || !err.to_string().contains("mode not supported") {
                    error!("  error processing beatmap \"{}\": {:#}", bm_name, err);
                }
            }
        }
    }
    Ok(flat_simfiles)
}

fn process_beatmap(
    conf: &OsuLoad,
    bmset_path: &Path,
    bm_path: &Path,
    mut out: impl FnMut(Box<Simfile>),
) -> Result<()> {
    let bm = Beatmap::parse(conf.offset, bm_path).context("read/parse beatmap file")?;
    ensure!(
        bm.mode == osufile::MODE_MANIA,
        "mode not supported ({}) only mania (3) is currently supported",
        bm.mode
    );
    let key_count = bm.circle_size.round();
    ensure!(
        key_count.is_finite() && key_count >= 0. && key_count < 128.,
        "invalid keycount {}",
        key_count
    );
    let mut first_tp = bm
        .timing_points
        .first()
        .ok_or(anyhow!("no timing points"))?
        .clone();
    ensure!(
        first_tp.beat_len > 0.,
        "beatLength of first timing point must be positive (is {})",
        first_tp.beat_len
    );
    struct ConvCtx<'a> {
        next_idx: usize,
        cur_tp: TimingPoint,
        cur_beat: BeatPos,
        cur_beat_nonapproxed: f64,
        timing_points: &'a [TimingPoint],
        out_bpms: Vec<ControlPoint>,
        out_notes: Vec<Note>,
    }
    let mut conv = ConvCtx {
        next_idx: 1,
        cur_tp: first_tp.clone(),
        cur_beat: BeatPos::from(0.),
        cur_beat_nonapproxed: 0.,
        timing_points: &bm.timing_points[..],
        out_bpms: Vec::new(),
        out_notes: Vec::new(),
    };
    /// Convert from a point in time to a snapped beat number, taking into account changing BPM.
    /// Should never be called with a time smaller than the last call!
    fn get_beat(conv: &mut ConvCtx, time: f64) -> BeatPos {
        //Advance timing points
        while conv.next_idx < conv.timing_points.len() {
            let next_tp = &conv.timing_points[conv.next_idx];
            if next_tp.beat_len <= 0. {
                //Skip inherited timing points
            } else if time >= next_tp.time {
                //Advance to this timing point
                conv.cur_beat_nonapproxed +=
                    (next_tp.time - conv.cur_tp.time) / conv.cur_tp.beat_len;
                //Arbitrary round-to-beat because of broken beatmaps
                conv.cur_beat = BeatPos::from(conv.cur_beat_nonapproxed).round(2);
                conv.cur_tp = next_tp.clone();
                conv.out_bpms.push(ControlPoint {
                    beat: conv.cur_beat,
                    beat_len: conv.cur_tp.beat_len / 1000.,
                });
            } else {
                //Still within the current timing point
                break;
            }
            conv.next_idx += 1;
        }
        //Use the current timing point to determine note beat
        conv.cur_beat + BeatPos::from((time - conv.cur_tp.time) / conv.cur_tp.beat_len)
    }
    // Adjust for hit objects that occur before the first timing point by adding another timing
    // point even earlier.
    if let Some(first_hit) = bm.hit_objects.first() {
        while first_hit.time < first_tp.time {
            first_tp.time -= first_tp.beat_len * first_tp.meter as f64;
        }
        conv.cur_tp = first_tp.clone();
        conv.out_bpms.push(ControlPoint {
            beat: BeatPos::from(0.),
            beat_len: first_tp.beat_len / 1000.,
        });
    }
    // Add hit objects as measure objects, pushing out SM notedata on the fly.
    let mut pending_tails = Vec::new();
    let mut last_time = f64::NEG_INFINITY;
    for obj in bm.hit_objects.iter() {
        //Ensure objects increase monotonically in time
        ensure!(
            obj.time >= last_time,
            "hit object occurs before previous object"
        );
        last_time = obj.time;
        //Insert any pending long note tails
        pending_tails.retain(|&(time, key)| {
            if time <= obj.time {
                // Insert now
                let end_beat = get_beat(&mut conv, time);
                conv.out_notes.push(Note {
                    kind: Note::KIND_TAIL,
                    beat: end_beat,
                    key,
                });
                false
            } else {
                // Keep waiting
                true
            }
        });
        //Get data for this object
        let obj_beat = get_beat(&mut conv, obj.time);
        let obj_key = (obj.x * key_count / 512.).floor();
        ensure!(
            obj_key.is_finite() && obj_key as i32 >= 0 && (obj_key as i32) < key_count as i32,
            "invalid object x {} corresponding to key {}",
            obj.x,
            obj_key
        );
        let obj_key = obj_key as i32;
        //Act depending on object type
        if obj.ty & osufile::TYPE_HOLD != 0 {
            // Long note
            // Get the end time in millis
            let end_time = obj
                .extras
                .split(':')
                .next()
                .unwrap_or_default()
                .parse::<f64>()
                .map_err(|_| {
                    anyhow!(
                        "invalid hold note extras \"{}\", expected endTime",
                        obj.extras
                    )
                })?;
            // Leave it for later insertion at the correct time
            let insert_idx = pending_tails
                .iter()
                .position(|(t, _)| *t > end_time)
                .unwrap_or(pending_tails.len());
            pending_tails.insert(insert_idx, (end_time, obj_key));
            // Insert the long note head
            conv.out_notes.push(Note {
                kind: Note::KIND_HEAD,
                beat: obj_beat,
                key: obj_key,
            });
        } else if obj.ty & osufile::TYPE_HIT != 0 {
            // Hit note
            conv.out_notes.push(Note {
                kind: Note::KIND_HIT,
                beat: obj_beat,
                key: obj_key,
            });
        }
    }
    // Push out any pending long note tails
    for (time, key) in pending_tails {
        let end_beat = get_beat(&mut conv, time);
        conv.out_notes.push(Note {
            kind: '3',
            beat: end_beat,
            key,
        });
    }
    // Generate sample length from audio file
    let default_len = 60.;
    let sample_len = if bm.audio.is_empty() || !conf.query_audio_len {
        default_len
    } else {
        let audio_path = bmset_path.join(&bm.audio);
        let (len, result) = get_audio_len(&audio_path);
        if let Err(err) = result {
            warn!(
                "    warning: failed to get full audio length for \"{}\": {:#}",
                audio_path.display(),
                err
            );
        }
        (len - bm.preview_start / 1000.).max(10.)
    };
    // Create the final SM file in all supported gamemodes
    for gamemode in conf
        .gamemodes
        .iter()
        .copied()
        .filter(|gm| gm.key_count() == key_count as i32)
    {
        out(Box::new(Simfile {
            title: if conf.unicode {
                bm.title_unicode.clone()
            } else {
                bm.title.clone()
            },
            title_trans: bm.title.clone(),
            subtitle: bm.version.clone(),
            subtitle_trans: bm.version.clone(),
            artist: if conf.unicode {
                bm.artist_unicode.clone()
            } else {
                bm.artist.clone()
            },
            artist_trans: bm.artist.clone(),
            genre: String::new(),
            credit: bm.creator.clone(),
            banner: None,
            background: Some(
                if conf.video && !bm.video.is_empty() {
                    Some(bm.video.clone().into())
                } else {
                    None
                }
                .unwrap_or_else(|| bm.background.clone().into()),
            ),
            lyrics: None,
            cdtitle: None,
            music: Some(bm.audio.clone().into()),
            offset: first_tp.time / -1000.,
            bpms: conv.out_bpms.clone(),
            stops: vec![],
            sample_start: Some(bm.preview_start / 1000.),
            sample_len: Some(sample_len),
            gamemode,
            desc: bm.version.clone(),
            difficulty: Difficulty::Edit,
            difficulty_num: f64::NAN,
            radar: [0., 0., 0., 0., 0.],
            notes: conv.out_notes.clone(),
        }));
    }
    Ok(())
}

/// Get the length of an audio file in seconds.
fn get_audio_len(path: &Path) -> (f64, Result<()>) {
    let (len, result) = match mp3_duration::from_path(path) {
        Ok(len) => (len, Ok(())),
        Err(err) => (err.at_duration, Err(err.into())),
    };
    (len.as_secs_f64(), result)
}
