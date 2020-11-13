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
    /// What fraction of a beat do osu! timing points mark.
    /// Several alternatives can be given, which will be tried from first to last until there are
    /// no timing point conflicts or no more roundings are available.
    ///
    /// If no roundings are supplied, it is equivalent to `vec![0.]` (no rounding at all).
    pub rounding: Vec<f64>,
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
            rounding: vec![4., 1., 0.5, 0.25, 0.125, 0.],
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct OsuMania {
    pub into: BucketId,
    /// Whether to check the error in milliseconds introduced by the conversion/quantization.
    pub check_error: bool,
}

impl Default for OsuMania {
    fn default() -> Self {
        Self {
            into: default(),
            check_error: false,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct OsuStd {
    pub into: BucketId,
    /// How many keys to convert standard beatmaps into.
    /// `0` by default, which disables the standard gamemode parser.
    pub keycount: i32,
    /// Similar to `Rekey::weight_curve`.
    pub weight_curve: Vec<(f32, f32)>,
    /// A list of distances, where the first distance corresponds to 1 key, the second to 2 keys,
    /// etc...
    /// If a jump is over the specified distance, it maps to a chord with that amount of keys.
    pub dist_to_keycount: Vec<f64>,
    /// How many notes to generate per spinner spin.
    pub steps_per_spin: f64,
    /// The minimum length of a slider bounce (in beats).
    pub min_slider_bounce: f64,
}

impl Default for OsuStd {
    fn default() -> Self {
        Self {
            into: default(),
            keycount: 4,
            weight_curve: vec![(0., 1.), (0.4, 10.), (0.8, 200.), (1.4, 300.)],
            dist_to_keycount: vec![0., 200., 350., 450.],
            steps_per_spin: 1.,
            min_slider_bounce: 0.25,
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
    let mut by_depth: Vec<Vec<PathBuf>> = Vec::new();
    let mut randtrim = if conf.debug_allow_chance < 1. {
        Some(FastRng::seed_from_u64(conf.debug_allow_seed))
    } else {
        None
    };
    for entry in WalkDir::new(&conf.input).contents_first(true) {
        let entry = match entry {
            Ok(entry) => entry,
            Err(err) => {
                // `walkdir::Error::display` already displays the errored path, so no need to
                // include that info.
                warn!("failed to scan input directory: {:#}", err);
                continue;
            }
        };
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
                    match process_beatmapset(conf, store, entry.path(), &dir[..], on_bmset) {
                        Ok(()) => {}
                        Err(e) => {
                            error!(
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
    store: &mut SimfileStore,
    bmset_path: &Path,
    bm_paths: &[PathBuf],
    on_bmset: &mut dyn FnMut(&mut SimfileStore) -> Result<()>,
) -> Result<()> {
    info!("processing \"{}\":", bmset_path.display());
    //Parse and convert beatmaps
    let mut bmset_cache = BmsetCache::default();
    let mut by_mode = [Vec::new(), Vec::new(), Vec::new(), Vec::new()];
    for bm_path in bm_paths {
        let mut simfile_count = 0;
        let result = process_beatmap(conf, &mut bmset_cache, bmset_path, bm_path, |mode, sm| {
            simfile_count += 1;
            by_mode[mode].push(sm)
        });
        let bm_name = bm_path.file_name().unwrap_or_default().to_string_lossy();
        match result {
            Ok(()) => {
                debug!(
                    "  loaded beatmap \"{}\" successfully into {} simfiles",
                    bm_name, simfile_count,
                );
            }
            Err(err) => {
                if !conf.ignore_mode_errors || !err.to_string().contains("mode not supported") {
                    error!("  error processing beatmap \"{}\": {:#}", bm_name, err);
                }
            }
        }
    }
    //Report beatmap
    store.reset();
    store.global_set("root", conf.input.to_string());
    store.global_set(
        "base",
        bmset_path
            .to_str()
            .ok_or(anyhow!(
                "non utf-8 beatmapset path \"{}\"",
                bmset_path.display()
            ))?
            .to_string(),
    );
    for (mode, simfiles) in by_mode.iter_mut().enumerate() {
        if simfiles.is_empty() {
            continue;
        }
        let bucket = match mode as i32 {
            osufile::MODE_MANIA => &conf.mania.into,
            osufile::MODE_STD => &conf.standard.into,
            _ => panic!("mode {} is unimplemented", mode),
        };
        store.put(bucket, simfiles.drain(..));
    }
    on_bmset(store)?;
    Ok(())
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
    cur_tp: TimingPoint,
    rest_tp: &'a [TimingPoint],
    cur_time: f64,
    cur_beat: BeatPos,
    rounding: BeatPos,
    inherited_multiplier: f64,
    out_beatlen_range: (f64, f64),
    out_offset: f64,
    out_bpms: Vec<ControlPoint>,
    out_notes: Vec<Note>,
}
impl ConvCtx<'_> {
    fn new<'a>(conf: &OsuLoad, bm: &'a Beatmap) -> Result<ConvCtx<'a>> {
        //Find the last absolute timing point before the first hitobject
        //If there are no absolute timing points before it, use the first absolute timing point
        //If there are no absolute timing points, well, there is nothing to do
        let first_hit_time = bm
            .hit_objects
            .first()
            .map(|hit| hit.time)
            .unwrap_or(bm.timing_points[0].time);
        let first_tp_idx = {
            let mut first_noninherited = None;
            let mut last_before_start = None;
            for (idx, tp) in bm.timing_points.iter().enumerate() {
                if tp.beat_len > 0. {
                    first_noninherited.get_or_insert(idx);
                    if tp.time <= first_hit_time {
                        last_before_start = Some(idx);
                    }
                }
            }
            last_before_start.or(first_noninherited).ok_or_else(|| {
                anyhow!(
                    "no non-inherited timing points found (timing points: {:?})",
                    bm.timing_points
                )
            })?
        };

        //Match the time of the first timing point to the time of the first hitobject
        let first_tp = {
            let mut first_tp = bm.timing_points[first_tp_idx].clone();
            let round_to = first_tp.beat_len * first_tp.meter as f64;
            first_tp.time += ((first_hit_time - first_tp.time) / round_to).floor() * round_to;
            first_tp
        };
        trace!(
            "    first tp at {}, with time {}",
            first_tp_idx,
            first_tp.time
        );

        //Now figure out the rounding of these timing points
        let mut final_rounding = None;
        for &rounding in conf.rounding.iter() {
            let round_to = BeatPos::from(rounding);
            let mut cur_tp = &first_tp;
            let mut cur_beat = BeatPos::from(0.);
            let mut no_aliasing = true;
            for tp in bm.timing_points[first_tp_idx + 1..].iter() {
                if tp.beat_len > 0. {
                    let last_beat = cur_beat;
                    let beat_adv =
                        BeatPos::from((tp.time - cur_tp.time) / cur_tp.beat_len).round(round_to);
                    cur_beat += beat_adv;
                    //Make sure there is no aliasing
                    if tp.time != cur_tp.time && cur_beat == last_beat {
                        no_aliasing = false;
                        break;
                    }
                    cur_tp = tp;
                }
            }
            if no_aliasing {
                final_rounding = Some(round_to);
                break;
            }
        }
        let final_rounding = final_rounding.unwrap_or(BeatPos::from(0.));

        //Create first control point
        let first_controlpoint = ControlPoint {
            beat: BeatPos::from(0.),
            beat_len: first_tp.beat_len / 1000.,
        };

        //Create context object
        Ok(ConvCtx {
            rest_tp: &bm.timing_points[first_tp_idx + 1..],
            cur_time: first_tp.time,
            cur_beat: BeatPos::from(0.),
            rounding: final_rounding,
            inherited_multiplier: 1.,
            out_beatlen_range: (first_tp.beat_len, first_tp.beat_len),
            out_offset: first_tp.time / -1000.,
            out_bpms: vec![first_controlpoint],
            out_notes: Vec::new(),
            cur_tp: first_tp,
        })
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
        //Advance timing points
        while let Some(next_tp) = self.rest_tp.first() {
            if time >= next_tp.time {
                if next_tp.beat_len <= 0. {
                    //Inherited timing points are only cosmetic (and they alter slider lengths)
                    self.inherited_multiplier = next_tp.beat_len / -100.;
                } else {
                    //Advance to this timing point
                    let raw_beat_adv = (next_tp.time - self.cur_time) / self.cur_tp.beat_len;
                    let beat_adv = BeatPos::from_num_ceil(raw_beat_adv).ceil(self.rounding);
                    let tp_beat = self.cur_beat + beat_adv;
                    let mut tp_time = self.cur_time + beat_adv.as_num() * self.cur_tp.beat_len;
                    if (tp_time - next_tp.time).abs() >= 4. {
                        let last_beat = self
                            .out_notes
                            .last()
                            .map(|note| note.beat)
                            .unwrap_or(BeatPos::from(0.))
                            .max(self.cur_beat);
                        let pivot_max = self.cur_beat + BeatPos::from_num_ceil(raw_beat_adv);
                        let mut pivot = None;
                        for &beat_gap in [
                            BeatPos::from(1.),
                            BeatPos::from(0.5),
                            BeatPos::from(0.25),
                            BeatPos::from(1. / 8.),
                            BeatPos::from(1. / 16.),
                            BeatPos::EPSILON,
                        ]
                        .iter()
                        {
                            let potential_pivot = pivot_max.ceil(beat_gap) - beat_gap;
                            if potential_pivot >= last_beat {
                                pivot = Some(potential_pivot);
                                break;
                            }
                        }
                        if let Some(pivot) = pivot {
                            let target_time = next_tp.time - self.cur_time;
                            let time_to_pivot =
                                (pivot - self.cur_beat).as_num() * self.cur_tp.beat_len;
                            let consume_time = target_time - time_to_pivot;
                            let consume_beats = tp_beat - pivot;
                            let beat_len = consume_time / consume_beats.as_num();
                            self.out_bpms.push(ControlPoint {
                                beat: pivot,
                                beat_len: beat_len / 1000.,
                            });
                            tp_time = self.cur_time
                                + (pivot - self.cur_beat).as_num() * self.cur_tp.beat_len
                                + (tp_beat - pivot).as_num() * beat_len;
                            trace!(
                                "      corrected bpm by inserting {}ms/beat control point at beat {}",
                                beat_len,
                                pivot
                            );
                        } else {
                            warn!("found no bpm correction pivot for timing points {:?} -> {:?}: last_beat = {}, pivot_max = {}", self.cur_tp, next_tp, last_beat, pivot_max);
                        }
                    }
                    trace!("      advancing from timing point at beat {}, time {}, to beat {} ({:?} -> {:?})", self.cur_beat, self.cur_time, tp_beat, self.cur_tp, next_tp);
                    self.cur_beat = tp_beat;
                    self.cur_time = tp_time;
                    self.cur_tp = next_tp.clone();
                    self.inherited_multiplier = 1.;
                    self.out_bpms.push(ControlPoint {
                        beat: self.cur_beat,
                        beat_len: self.cur_tp.beat_len / 1000.,
                    });
                    self.out_beatlen_range.0 = self.out_beatlen_range.0.min(self.cur_tp.beat_len);
                    self.out_beatlen_range.1 = self.out_beatlen_range.1.max(self.cur_tp.beat_len);
                }
                self.rest_tp = &self.rest_tp[1..];
            } else {
                //Still within the current timing point
                break;
            }
        }
        //Use the current timing point to determine note beat
        //Do not use `cur_time`; it is only used as an error accumulator
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
                offset: self.out_offset,
                bpms: self.out_bpms.clone(),
                stops: vec![],
                sample_start: Some(bm.preview_start / 1000.),
                sample_len: Some(sample_len),
                display_bpm: if self.out_beatlen_range.0 == self.out_beatlen_range.1 {
                    DisplayBpm::Single(60000. / self.out_beatlen_range.0)
                } else {
                    //Use `.1` for the lower bound and `.0` for the higher bound, because lower
                    //beatlens imply higher BPMs
                    DisplayBpm::Range(
                        60000. / self.out_beatlen_range.1,
                        60000. / self.out_beatlen_range.0,
                    )
                },
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
    let mut conv = ConvCtx::new(conf, &bm)?;
    let key_count = match bm.mode {
        osufile::MODE_MANIA => process_mania(conf, &bm, &mut conv)?,
        osufile::MODE_STD => process_standard(conf, &bm, &mut conv)?,
        osufile::MODE_CATCH => bail!("mode not supported: catch the beat"),
        osufile::MODE_TAIKO => bail!("mode not supported: taiko"),
        unknown => bail!("mode not supported: unknown osu! gamemode {}", unknown),
    };
    //Finish up
    if key_count != 0 {
        conv.finish(
            conf,
            bmset_cache,
            bmset_path,
            bm_path,
            &bm,
            key_count,
            |sm| out(bm.mode as usize, sm),
        )?;
    }
    Ok(())
}

fn process_mania(conf: &OsuLoad, bm: &Beatmap, conv: &mut ConvCtx) -> Result<i32> {
    let key_count = bm.circle_size.round();
    ensure!(
        key_count.is_finite() && key_count >= 0. && key_count < 128.,
        "invalid keycount {}",
        key_count
    );
    trace!(
        "    processing {} osu!mania ({}K) hitobjects",
        bm.hit_objects.len(),
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
    //Check precision
    if conf.mania.check_error {
        let sm = Simfile {
            title: default(),
            artist: default(),
            subtitle: default(),
            title_trans: default(),
            artist_trans: default(),
            subtitle_trans: default(),
            genre: default(),
            credit: default(),
            banner: default(),
            background: default(),
            lyrics: default(),
            cdtitle: default(),
            music: default(),
            offset: conv.out_offset,
            bpms: conv.out_bpms.clone(),
            stops: default(),
            sample_start: default(),
            sample_len: default(),
            display_bpm: DisplayBpm::Random,
            gamemode: Gamemode::DanceSingle,
            desc: default(),
            difficulty: Difficulty::Edit,
            difficulty_num: f64::NAN,
            radar: default(),
            notes: vec![],
        };
        let mut notes = conv.out_notes.clone();
        let mut check_dist = |key: i32, kind: char, time: f64| -> Result<f64> {
            let (note, dist) = notes
                .iter_mut()
                .filter_map(|note| {
                    if note.key == key && note.kind == kind {
                        let sm_time = sm.beat_to_time().beat_to_time(note.beat) * 1000.;
                        let dist = (time - sm_time).abs();
                        Some((note, dist))
                    } else {
                        None
                    }
                })
                .min_by_key(|(_n, dist)| SortableFloat(*dist))
                .ok_or_else(|| anyhow!("more osu notes than sm notes"))?;
            note.key = -1;
            Ok(dist)
        };
        let mut max_dist = 0f64;
        for obj in bm.hit_objects.iter() {
            //Get key
            let obj_key = (obj.x * key_count / 512.).floor();
            ensure!(
                obj_key.is_finite() && obj_key as i32 >= 0 && (obj_key as i32) < key_count as i32,
                "invalid object x {} corresponding to key {}",
                obj.x,
                obj_key
            );
            let obj_key = obj_key as i32;
            //Check note start
            max_dist = max_dist.max(check_dist(
                obj_key,
                if obj.ty & osufile::TYPE_LONG == 0 {
                    Note::KIND_HIT
                } else {
                    Note::KIND_HEAD
                },
                obj.time,
            )?);
            //Also check longnote ends
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
                //Check note end
                max_dist = max_dist.max(check_dist(obj_key, Note::KIND_TAIL, end_time)?);
            }
        }
        ensure!(
            notes.iter().all(|note| note.key == -1),
            "more sm notes than osu notes (leftovers: {:?})",
            notes
                .iter()
                .filter(|note| note.key != -1)
                .collect::<Vec<_>>()
        );
        trace!("      max error in milliseconds: {}", max_dist);
    }
    Ok(key_count as i32)
}

fn process_standard(conf: &OsuLoad, bm: &Beatmap, conv: &mut ConvCtx) -> Result<i32> {
    use crate::node::rekey::KeyAlloc;

    let key_count = conf.standard.keycount;
    if key_count == 0 {
        //Disable the standard parser
        return Ok(0);
    }
    ensure!(key_count > 0, "keycount must be positive");
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

    trace!(
        "    processing {} osu!standard hitobjects into {}K simfile",
        bm.hit_objects.len(),
        key_count
    );

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
                    if let Some((pos, out_key)) =
                        key_alloc.alloc_idx(&tmp_choose_vec, obj.time / 1000., &mut rng)
                    {
                        tmp_choose_vec.swap_remove(pos);
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
                let mut slides = extras
                    .next()
                    .unwrap_or_default()
                    .parse::<i32>()
                    .map_err(|_| {
                        anyhow!("invalid spinner extras \"{}\", expected slides", obj.extras)
                    })?
                    .max(1) as usize;
                let length_pixels =
                    extras
                        .next()
                        .unwrap_or_default()
                        .parse::<f64>()
                        .map_err(|_| {
                            anyhow!("invalid spinner extras \"{}\", expected length", obj.extras)
                        })?;
                //The length of _the entire_ slider in milliseconds, factoring in multiple slides
                //Note that only the beat length of the starting timing point is considered, to be
                //consistent with how osu! works does it.
                let slider_len = slides as f64 * length_pixels / (100. * bm.slider_multiplier)
                    * (conv.cur_tp.beat_len * conv.inherited_multiplier);
                //Convert the length to beats
                let beat_len = conv.get_beat(obj.time + slider_len) - beat;
                if beat_len.as_num() / (slides as f64) < conf.standard.min_slider_bounce {
                    slides = (beat_len.as_num() / conf.standard.min_slider_bounce).round() as usize;
                }
                //Divide the slider in potentially several slides
                let mut cur_slide_start = beat;
                for slide_idx in 0..slides {
                    //Add head notes
                    tmp_choose_vec.clear();
                    tmp_choose_vec.extend(0..key_count);
                    let mut available_keys = key_count;
                    for _ in 0..keys {
                        if let Some((pos, out_key)) = key_alloc.alloc_idx(
                            &tmp_choose_vec[..available_keys],
                            obj.time / 1000.,
                            &mut rng,
                        ) {
                            tmp_choose_vec[pos..].rotate_left(1);
                            available_keys -= 1;
                            //Push head note
                            conv.push_note(cur_slide_start, out_key as i32, Note::KIND_HEAD);
                        } else {
                            break;
                        }
                    }
                    //Advance beat
                    let head_beat = cur_slide_start;
                    cur_slide_start = beat
                        + BeatPos::from((slide_idx + 1) as f64 / slides as f64 * beat_len.as_num());
                    if cur_slide_start == head_beat {
                        error!("beat length = {}, slides = {}", beat_len, slides);
                    }
                    //Add tails
                    for i in available_keys..key_count {
                        conv.push_note(cur_slide_start, tmp_choose_vec[i] as i32, Note::KIND_TAIL);
                    }
                }
                //Use the last control point as the final slider position
                //Kinda hacky, but very simple
                let mut end_pos = (obj.x, obj.y);
                //Make sure the end position is only used if the slider does not roll back to its
                //initial position
                if slides % 2 == 1 {
                    //Parse curve
                    let mut curve = curve.split('|');
                    let _curve_ty = curve.next().unwrap();
                    let last_point = curve.next_back().unwrap_or_default();
                    let mut point = last_point.split(':');
                    let x = point
                        .next()
                        .unwrap_or_default()
                        .parse::<f64>()
                        .map_err(|_| {
                            anyhow!("invalid slider point \"{}\", expected x", last_point)
                        })?;
                    let y = point
                        .next()
                        .unwrap_or_default()
                        .parse::<f64>()
                        .map_err(|_| {
                            anyhow!("invalid slider point \"{}\", expected y", last_point)
                        })?;
                    end_pos = (x, y);
                }
                last_pos = Some(end_pos);
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

    Ok(key_count as i32)
}
