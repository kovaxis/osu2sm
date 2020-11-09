//! Take an osu! input directory and parse its beatmaps.

use crate::node::prelude::*;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct OsuLoad {
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
    /// Options for mania beatmaps.
    pub mania: OsuMania,
    /// Options for beatmaps converted from osu!standard.
    pub standard: OsuStd,
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
            mania: default(),
            standard: default(),
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

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct OsuMania {
    into: BucketId,
}

impl Default for OsuMania {
    fn default() -> Self {
        Self { into: default() }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct OsuStd {
    into: BucketId,
    keycount: i32,
    weight_curve: Vec<(f32, f32)>,
    dist_to_keycount: Vec<f64>,
    /// How many notes to generate per spinner spin.
    steps_per_spin: f64,
}

impl Default for OsuStd {
    fn default() -> Self {
        Self {
            into: default(),
            keycount: 4,
            weight_curve: vec![(0., 1.), (0.4, 10.), (0.8, 200.), (1.4, 300.)],
            dist_to_keycount: vec![0., 200., 350., 450.],
            steps_per_spin: 1.,
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
        Box::new(
            iter::once((BucketKind::Output, &mut self.mania.into))
                .chain(iter::once((BucketKind::Output, &mut self.standard.into))),
        )
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
                        Ok(mut simfiles) => {
                            for (mode, simfiles) in simfiles.iter_mut().enumerate() {
                                if simfiles.is_empty() {
                                    continue;
                                }
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
                                let bucket = match mode as i32 {
                                    osufile::MODE_MANIA => &conf.mania.into,
                                    osufile::MODE_STD => &conf.standard.into,
                                    _ => panic!("mode {} is unimplemented", mode),
                                };
                                store.put(bucket, simfiles.drain(..));
                                on_bmset(store)?;
                            }
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
) -> Result<[Vec<Box<Simfile>>; 4]> {
    info!("processing \"{}\":", bmset_path.display());
    //Parse and convert beatmaps
    let mut bmset_cache = BmsetCache::default();
    let mut flat_simfiles = [Vec::new(), Vec::new(), Vec::new(), Vec::new()];
    for bm_path in bm_paths {
        let mut simfile_count = 0;
        let result = process_beatmap(conf, &mut bmset_cache, bmset_path, bm_path, |mode, sm| {
            simfile_count += 1;
            flat_simfiles[mode].push(sm)
        });
        let bm_name = bm_path.file_name().unwrap_or_default().to_string_lossy();
        match result {
            Ok(()) => {
                if simfile_count <= 0 {
                    warn!("  beatmap \"{}\" loaded, but produced no simfiles (check `OsuLoad` gamemodes)", bm_name);
                } else {
                    debug!(
                        "  loaded beatmap \"{}\" successfully into {} simfiles",
                        bm_name, simfile_count,
                    );
                }
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

#[derive(Default)]
struct BmsetCache {
    audio_len: HashMap<PathBuf, f64>,
}
impl BmsetCache {
    /// Get the length of an audio file in seconds.
    fn get_audio_len(&mut self, path: &Path) -> (f64, Result<()>) {
        let mut result = Ok(());
        let len = match self.audio_len.get(path) {
            Some(len) => *len,
            None => {
                let len = match mp3_duration::from_path(path) {
                    Ok(len) => len,
                    Err(err) => {
                        let len = err.at_duration;
                        result = Err(err.into());
                        len
                    }
                }
                .as_secs_f64();
                self.audio_len.insert(path.to_path_buf(), len);
                len
            }
        };
        (len, result)
    }
}

struct ConvCtx<'a> {
    next_idx: usize,
    first_tp: TimingPoint,
    cur_tp: TimingPoint,
    cur_beat: BeatPos,
    cur_beat_nonapproxed: f64,
    inherited_multiplier: f64,
    last_time: f64,
    timing_points: &'a [TimingPoint],
    out_bpms: Vec<ControlPoint>,
    out_notes: Vec<Note>,
}
impl ConvCtx<'_> {
    fn new(bm: &Beatmap) -> Result<ConvCtx> {
        let first_tp = bm
            .timing_points
            .first()
            .ok_or(anyhow!("no timing points"))?
            .clone();
        ensure!(
            first_tp.beat_len > 0.,
            "beatLength of first timing point must be positive (is {})",
            first_tp.beat_len
        );
        let mut conv = ConvCtx {
            next_idx: 1,
            cur_tp: first_tp.clone(),
            first_tp: first_tp,
            cur_beat: BeatPos::from(0.),
            cur_beat_nonapproxed: 0.,
            inherited_multiplier: 1.,
            last_time: f64::NEG_INFINITY,
            timing_points: &bm.timing_points[..],
            out_bpms: Vec::new(),
            out_notes: Vec::new(),
        };
        // Adjust for hit objects that occur before the first timing point by adding another timing
        // point even earlier.
        if let Some(first_hit) = bm.hit_objects.first() {
            while first_hit.time < conv.first_tp.time {
                conv.first_tp.time -= conv.first_tp.beat_len * conv.first_tp.meter as f64;
            }
            conv.cur_tp = conv.first_tp.clone();
            conv.out_bpms.push(ControlPoint {
                beat: BeatPos::from(0.),
                beat_len: conv.first_tp.beat_len / 1000.,
            });
        }
        Ok(conv)
    }

    /// Convert from a point in time to a snapped beat number, taking into account changing BPM.
    /// Should never be called with a time smaller than the last call!
    fn get_beat(&mut self, time: f64) -> BeatPos {
        /*
        ensure!(
            time >= self.last_time,
            "object must monotonically increase in time",
        );
        */
        self.last_time = time;
        //Advance timing points
        while self.next_idx < self.timing_points.len() {
            let next_tp = &self.timing_points[self.next_idx];
            if next_tp.beat_len <= 0. {
                //Skip inherited timing points
                self.inherited_multiplier = next_tp.beat_len / -100.;
            } else if time >= next_tp.time {
                //Advance to this timing point
                self.cur_beat_nonapproxed +=
                    (next_tp.time - self.cur_tp.time) / self.cur_tp.beat_len;
                //Arbitrary round-to-beat because of broken beatmaps
                self.cur_beat = BeatPos::from(self.cur_beat_nonapproxed).round(2);
                self.cur_tp = next_tp.clone();
                self.inherited_multiplier = 1.;
                self.out_bpms.push(ControlPoint {
                    beat: self.cur_beat,
                    beat_len: self.cur_tp.beat_len / 1000.,
                });
            } else {
                //Still within the current timing point
                break;
            }
            self.next_idx += 1;
        }
        //Use the current timing point to determine note beat
        self.cur_beat + BeatPos::from((time - self.cur_tp.time) / self.cur_tp.beat_len)
    }

    /// Add an output note.
    fn push_note(&mut self, beat: BeatPos, key: i32, kind: char) {
        self.out_notes.push(Note { beat, key, kind });
    }

    /// Output the final simfile in all supported gamemodes.
    fn finish(
        self,
        conf: &OsuLoad,
        bmset_cache: &mut BmsetCache,
        bmset_path: &Path,
        bm_path: &Path,
        bm: &Beatmap,
        key_count: i32,
        mut out: impl FnMut(Box<Simfile>),
    ) -> Result<()> {
        // Generate sample length from audio file
        let default_len = 60.;
        let sample_len = if bm.audio.is_empty() || !conf.query_audio_len {
            default_len
        } else {
            let audio_path = bmset_path.join(&bm.audio);
            let (len, result) = bmset_cache.get_audio_len(&audio_path);
            if let Err(err) = result {
                warn!(
                    "    failed to get full audio length for \"{}\": {:#}",
                    audio_path.display(),
                    err
                );
            }
            (len - bm.preview_start / 1000.).max(10.)
        };
        // Create the final SM file in all supported gamemodes
        let mut at_least_one = false;
        for gamemode in conf
            .gamemodes
            .iter()
            .copied()
            .filter(|gm| gm.key_count() == key_count as i32)
        {
            at_least_one = true;
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
                offset: self.first_tp.time / -1000.,
                bpms: self.out_bpms.clone(),
                stops: vec![],
                sample_start: Some(bm.preview_start / 1000.),
                sample_len: Some(sample_len),
                gamemode,
                desc: bm.version.clone(),
                difficulty: Difficulty::Edit,
                difficulty_num: f64::NAN,
                radar: [0., 0., 0., 0., 0.],
                notes: self.out_notes.clone(),
            }));
        }
        if !at_least_one {
            warn!(
                "  beatmap \"{}\" parsed correctly, but there are no compatible gamemodes with keycount {}",
                bm_path.display(),
                key_count
            );
        }
        Ok(())
    }
}

fn process_beatmap(
    conf: &OsuLoad,
    bmset_cache: &mut BmsetCache,
    bmset_path: &Path,
    bm_path: &Path,
    mut out: impl FnMut(usize, Box<Simfile>),
) -> Result<()> {
    let bm = Beatmap::parse(conf.offset, bm_path).context("read/parse beatmap file")?;
    let mut conv = ConvCtx::new(&bm)?;
    let key_count = match bm.mode {
        osufile::MODE_MANIA => process_mania(conf, &bm, &mut conv)?,
        osufile::MODE_STD => process_standard(conf, &bm, &mut conv)?,
        osufile::MODE_CATCH => bail!("mode not supported: catch the beat"),
        osufile::MODE_TAIKO => bail!("mode not supported: taiko"),
        unknown => bail!("mode not supported: unknown osu! gamemode {}", unknown),
    };
    //Finish up
    conv.finish(
        conf,
        bmset_cache,
        bmset_path,
        bm_path,
        &bm,
        key_count,
        |sm| out(bm.mode as usize, sm),
    )
}

fn process_mania(_conf: &OsuLoad, bm: &Beatmap, conv: &mut ConvCtx) -> Result<i32> {
    let key_count = bm.circle_size.round();
    ensure!(
        key_count.is_finite() && key_count >= 0. && key_count < 128.,
        "invalid keycount {}",
        key_count
    );
    //Keep track of pending long note tails, and add them when it's time
    let mut pending_tails = Vec::new();
    //Go through every osu! hit object
    for obj in bm.hit_objects.iter() {
        //Insert any pending long note tails
        pending_tails.retain(|&(time, key)| {
            if time <= obj.time {
                //Insert now
                let end_beat = conv.get_beat(time);
                conv.push_note(end_beat, key, Note::KIND_TAIL);
                false
            } else {
                //Keep waiting
                true
            }
        });
        //Get data for this object
        let obj_beat = conv.get_beat(obj.time);
        let obj_key = (obj.x * key_count / 512.).floor();
        ensure!(
            obj_key.is_finite() && obj_key as i32 >= 0 && (obj_key as i32) < key_count as i32,
            "invalid object x {} corresponding to key {}",
            obj.x,
            obj_key
        );
        let obj_key = obj_key as i32;
        //Act depending on object type
        if obj.ty & osufile::TYPE_LONG != 0 {
            //Long note
            //Get the end time in millis
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
            //Leave it for later insertion at the correct time
            let insert_idx = pending_tails
                .iter()
                .position(|(t, _)| *t > end_time)
                .unwrap_or(pending_tails.len());
            pending_tails.insert(insert_idx, (end_time, obj_key));
            //Insert the long note head
            conv.push_note(obj_beat, obj_key, Note::KIND_HEAD);
        } else if obj.ty & osufile::TYPE_HIT != 0 {
            //Hit note
            conv.push_note(obj_beat, obj_key, Note::KIND_HIT);
        }
    }
    // Push out any pending long note tails
    for (time, key) in pending_tails {
        let end_beat = conv.get_beat(time);
        conv.push_note(end_beat, key, Note::KIND_TAIL);
    }
    Ok(key_count as i32)
}

fn process_standard(conf: &OsuLoad, bm: &Beatmap, conv: &mut ConvCtx) -> Result<i32> {
    use crate::node::remap::KeyAlloc;

    let key_count = conf.standard.keycount;
    ensure!(key_count > 0, "keycount must be greater than 0");
    let key_count = key_count as usize;
    let mut key_alloc = KeyAlloc::new(&conf.standard.weight_curve, key_count);
    let mut rng = FastRng::seed_from_u64(fxhash::hash64(&(
        &bm.title,
        &bm.artist,
        &bm.version,
        bm.set_id,
        bm.id,
        "osuload-std",
    )));

    let get_key_count = |last_pos: Option<(f64, f64)>, cur_pos: (f64, f64)| -> usize {
        let (x, y) = cur_pos;
        let (last_x, last_y) = last_pos.unwrap_or(cur_pos);
        let (dx, dy) = (x - last_x, y - last_y);
        let dist_sq = dx * dx + dy * dy;
        conf.standard
            .dist_to_keycount
            .iter()
            .rposition(|&min_dist| dist_sq >= min_dist * min_dist)
            .map(|idx| idx + 1)
            .unwrap_or(0)
    };

    let mut tmp_choose_vec = Vec::with_capacity(key_count);
    let mut last_pos = None;
    for obj in bm.hit_objects.iter() {
        let beat = conv.get_beat(obj.time);
        if obj.ty & osufile::TYPE_HIT != 0 {
            //Create a chord from a single hit
            let keys = get_key_count(last_pos, (obj.x, obj.y));
            if keys > 0 {
                tmp_choose_vec.clear();
                tmp_choose_vec.extend(0..key_count);
                for _ in 0..keys {
                    if let Some(out_key) =
                        key_alloc.alloc(&tmp_choose_vec, obj.time / 1000., &mut rng)
                    {
                        let pos = tmp_choose_vec.iter().position(|&k| k == out_key).unwrap();
                        tmp_choose_vec.remove(pos);
                        conv.push_note(beat, out_key as i32, Note::KIND_HIT);
                    } else {
                        break;
                    }
                }
                last_pos = Some((obj.x, obj.y));
            }
        } else if obj.ty & osufile::TYPE_SLIDER != 0 {
            //Create a hold chord from a single slider
            let keys = get_key_count(last_pos, (obj.x, obj.y));
            if keys > 0 {
                //Parse slider properties
                let mut extras = obj.extras.split(',');
                let curve = extras.next().unwrap_or_default();
                let slides = extras
                    .next()
                    .unwrap_or_default()
                    .parse::<u32>()
                    .map_err(|_| {
                        anyhow!("invalid spinner extras \"{}\", expected slides", obj.extras)
                    })? as usize;
                let length_pixels =
                    extras
                        .next()
                        .unwrap_or_default()
                        .parse::<f64>()
                        .map_err(|_| {
                            anyhow!("invalid spinner extras \"{}\", expected length", obj.extras)
                        })?;
                //Do each slider section
                let beat_len = BeatPos::from(
                    length_pixels / (100. * bm.slider_multiplier) * conv.inherited_multiplier,
                );
                let mut next_start_beat = beat;
                for _ in 0..slides {
                    //Add head notes
                    tmp_choose_vec.clear();
                    tmp_choose_vec.extend(0..key_count);
                    let mut available_keys = key_count;
                    for _ in 0..keys {
                        if let Some(out_key) = key_alloc.alloc(
                            &tmp_choose_vec[..available_keys],
                            obj.time / 1000.,
                            &mut rng,
                        ) {
                            let pos = tmp_choose_vec[..available_keys]
                                .iter()
                                .position(|&k| k == out_key)
                                .unwrap();
                            tmp_choose_vec[pos..].rotate_left(1);
                            available_keys -= 1;
                            //Push head note
                            conv.push_note(next_start_beat, out_key as i32, Note::KIND_HEAD);
                        } else {
                            break;
                        }
                    }
                    //Advance beat
                    next_start_beat += beat_len;
                    //Add tails
                    for i in available_keys..key_count {
                        conv.push_note(next_start_beat, tmp_choose_vec[i] as i32, Note::KIND_TAIL);
                    }
                }
                //TODO: Use actual curve to determine the end of the slider
                last_pos = Some((obj.x, obj.y));
            }
        } else if obj.ty & osufile::TYPE_SPINNER != 0 {
            //Convert spinners to stairs
            //Parse spinner endtime
            let end_time = obj
                .extras
                .split(',')
                .next()
                .unwrap_or_default()
                .parse::<f64>()
                .map_err(|_| {
                    anyhow!(
                        "invalid spinner extras \"{}\", expected endTime",
                        obj.extras
                    )
                })?;
            let end_beat = conv.get_beat(end_time);
            //Taken from the osu! wiki
            let spins_per_sec = if bm.overall_difficulty < 5. {
                5. - 2. * (5. - bm.overall_difficulty) / 5.
            } else {
                5. + 2.5 * (bm.overall_difficulty - 5.) / 5.
            };
            let spins = (end_time - obj.time) / 1000. * spins_per_sec;
            let steps_approx = (spins * conf.standard.steps_per_spin).max(1.);
            let beat_step = BeatPos::from(
                2f64.powi(
                    ((end_beat - beat).as_num() / steps_approx)
                        .max(1. / 16.)
                        .log2()
                        .round() as i32,
                ),
            );
            //Create stair steps
            tmp_choose_vec.clear();
            tmp_choose_vec.extend(0..key_count);
            let mut next_key = key_alloc
                .alloc(&tmp_choose_vec, obj.time / 1000., &mut rng)
                .unwrap() as i32;
            let dir = if rng.gen() { 1 } else { -1 };
            let mut next_beat = beat;
            while next_beat <= end_beat {
                conv.push_note(next_beat, next_key, Note::KIND_HIT);
                next_beat += beat_step;
                next_key = (next_key + dir).rem_euclid(key_count as i32);
            }
            last_pos = None;
        }
    }

    trace!(
        "generated {} osu! standard notes from {} hitobjects",
        conv.out_notes.len(),
        bm.hit_objects.len()
    );

    Ok(key_count as i32)
}
