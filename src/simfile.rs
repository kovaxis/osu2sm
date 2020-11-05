//! Create and write stepmania simfiles.

use crate::prelude::*;

/// Forced to be 4 by the godlike simfile format.
const BEATS_IN_MEASURE: i32 = 4;

#[derive(Debug, Clone)]
pub struct Simfile {
    pub title: String,
    pub subtitle: String,
    pub artist: String,
    pub title_trans: String,
    pub subtitle_trans: String,
    pub artist_trans: String,
    pub genre: String,
    pub credit: String,
    pub banner: Option<PathBuf>,
    pub background: Option<PathBuf>,
    pub lyrics: Option<PathBuf>,
    pub cdtitle: Option<PathBuf>,
    pub music: Option<PathBuf>,
    pub offset: f64,
    pub bpms: Vec<ControlPoint>,
    pub stops: Vec<(f64, f64)>,
    pub sample_start: Option<f64>,
    pub sample_len: Option<f64>,
    pub gamemode: Gamemode,
    pub desc: String,
    pub difficulty: Difficulty,
    pub difficulty_num: f64,
    pub radar: [f64; 5],
    pub notes: Vec<Note>,
}
impl Simfile {
    pub fn save<'a>(path: &Path, simfiles: impl IntoIterator<Item = &'a Simfile>) -> Result<()> {
        let mut simfiles = simfiles.into_iter();
        let main_sm = simfiles.next().ok_or(anyhow!("zero simfiles supplied"))?;
        let mut file = BufWriter::new(File::create(path).context("create file")?);
        fn as_utf8<'a>(path: &'a Option<PathBuf>, name: &str) -> Result<&'a str> {
            path.as_deref()
                .unwrap_or_else(|| "".as_ref())
                .to_str()
                .ok_or_else(|| anyhow!("non-utf8 {}", name))
        }
        write!(
            file,
            r#"
// Simfile converted from osu! automatically using `osu2sm` by negamartin
#TITLE:{title};
#SUBTITLE:{subtitle};
#ARTIST:{artist};
#TITLETRANSLIT:{title_t};
#SUBTITLETRANSLIT:{subtitle_t};
#ARTISTTRANSLIT:{artist_t};
#GENRE:{genre};
#CREDIT:{credit};
#BANNER:{banner};
#BACKGROUND:{bg};
#LYRICSPATH:{lyrics};
#CDTITLE:{cdtitle};
#MUSIC:{music};
#OFFSET:{offset};
#SAMPLESTART:{sample_start};
#SAMPLELENGTH:{sample_len};
#SELECTABLE:YES;
#BPMS:{bpms};
#STOPS:;
#BGCHANGES:;
#KEYSOUNDS:;
#ATTACKS:;
"#,
            title = main_sm.title,
            subtitle = main_sm.subtitle,
            artist = main_sm.artist,
            title_t = main_sm.title_trans,
            subtitle_t = main_sm.subtitle_trans,
            artist_t = main_sm.artist_trans,
            genre = main_sm.genre,
            credit = main_sm.credit,
            banner = as_utf8(&main_sm.banner, "BANNER")?,
            bg = as_utf8(&main_sm.background, "BACKGROUND")?,
            lyrics = as_utf8(&main_sm.lyrics, "LYRICSPATH")?,
            cdtitle = as_utf8(&main_sm.cdtitle, "CDTITLE")?,
            music = as_utf8(&main_sm.music, "MUSIC")?,
            offset = main_sm.offset,
            sample_start = main_sm
                .sample_start
                .map(|s| format!("{}", s))
                .unwrap_or_else(String::new),
            sample_len = main_sm
                .sample_len
                .map(|l| format!("{}", l))
                .unwrap_or_else(String::new),
            bpms = {
                let mut bpms = String::new();
                let mut first = true;
                for point in main_sm.bpms.iter() {
                    if first {
                        first = false;
                    } else {
                        bpms.push(',');
                    }
                    write!(bpms, "{}={}", point.beat.as_num(), point.bpm()).unwrap();
                }
                bpms
            },
        )?;
        for sm in iter::once(main_sm).chain(simfiles) {
            write!(
                file,
                r#"
#NOTES:
    {gamemode}:
    {desc}:
    {diff_name}:
    {diff_num}:
    {radar0}, {radar1}, {radar2}, {radar3}, {radar4}:"#,
                gamemode = sm.gamemode.id(),
                desc = sm.desc,
                diff_name = sm.difficulty.name(),
                diff_num = sm.difficulty_num,
                radar0 = sm.radar[0],
                radar1 = sm.radar[1],
                radar2 = sm.radar[2],
                radar3 = sm.radar[3],
                radar4 = sm.radar[4],
            )?;
            write_notedata(&mut file, &sm)?;
            write!(file, ";")?;
        }
        Ok(())
    }

    pub fn file_deps(&self) -> impl Iterator<Item = &Path> {
        self.banner
            .as_deref()
            .into_iter()
            .chain(self.background.as_deref().into_iter())
            .chain(self.lyrics.as_deref().into_iter())
            .chain(self.cdtitle.as_deref().into_iter())
            .chain(self.music.as_deref().into_iter())
    }

    /// Get the estimated difficulty of a certain chart.
    pub fn difficulty(&self) -> f64 {
        fn adapt_range(src: (f64, f64), dst: (f64, f64), val: f64) -> f64 {
            dst.0 + (val - src.0) / (src.1 - src.0) * (dst.1 - dst.0)
        }
        let diff = adapt_range((6., 14.), (1., 12.), (self.notes.len() as f64).log2());
        diff.max(1.)
    }

    /// Osu allows two notes at the same time and key, but the `.sm` format disallows this.
    ///
    /// Having two notes at the exact same location is usually wrong, except for the tail -> head
    /// or tail -> hit case (where a note ends and another note immediately starts).
    /// In order to fix this, if there is a tail and afterwards at the exact same beat and key
    /// there is another note, the tail is moved back a little.
    /// Note that this requires sorting the notes if any is moved.
    pub fn fix_tails(&mut self) -> Result<()> {
        let mut cur_beat = BeatPos::from(0.);
        let mut cur_beat_first_note = 0;
        for i in 0..self.notes.len() {
            let note = &self.notes[i];
            if note.beat > cur_beat {
                cur_beat_first_note = i;
                cur_beat = note.beat;
            }
            if note.is_tail() {
                if self.notes[i + 1..]
                    .iter()
                    .take_while(|next_n| next_n.beat == cur_beat)
                    .any(|next_n| next_n.key == note.key)
                {
                    //Move back by the smallest beat unit, and to the previous beat
                    self.notes[i].beat -= BeatPos::EPSILON;
                    self.notes[cur_beat_first_note..i + 1].rotate_right(1);
                }
            }
        }
        Ok(())
    }
}

fn write_measure(
    file: &mut impl Write,
    key_count: i32,
    measure_idx: usize,
    measure_start: BeatPos,
    notes: &[Note],
) -> Result<()> {
    //Extract largest simplified denominator, in prime-factorized form.
    //To obtain the actual number from prime-factorized form, use 2^pf[0] * 3^pf[1]
    fn get_denom(mut num: i32) -> [u32; 2] {
        let mut den = BeatPos::FIXED_POINT;
        let mut simplify_by = [0; 2];
        for (idx, &factor) in [2, 3].iter().enumerate() {
            while num % factor == 0 && den % factor == 0 {
                num /= factor;
                den /= factor;
                simplify_by[idx] += 1;
            }
        }
        simplify_by
    }
    let simplify_by = if notes.is_empty() {
        BeatPos::FIXED_POINT
    } else {
        let mut max_simplify_by = [u32::MAX; 2];
        for note in notes {
            let rel_pos = note.beat - measure_start;
            ensure!(
                rel_pos >= BeatPos::from(0.),
                "handed a note that starts before the measure start ({} < {})",
                note.beat,
                measure_start
            );
            let simplify_by = get_denom(rel_pos.frac);
            for (max_exp, exp) in max_simplify_by.iter_mut().zip(simplify_by.iter()) {
                *max_exp = u32::min(*max_exp, *exp);
            }
        }
        2i32.pow(max_simplify_by[0]) * 3i32.pow(max_simplify_by[1])
    };
    let rows_per_beat = BeatPos::FIXED_POINT / simplify_by;
    //Output 4x this amount of rows (if 4 beats in measure)
    let mut out_measure =
        vec![b'0'; (BEATS_IN_MEASURE * rows_per_beat) as usize * key_count as usize];
    for note in notes {
        let rel_pos = note.beat - measure_start;
        let idx = (rel_pos.frac / simplify_by) as usize;
        ensure!(
            rel_pos.frac % simplify_by == 0,
            "incorrect simplify_by ({} % {} == {} != 0)",
            rel_pos,
            simplify_by,
            rel_pos.frac % simplify_by
        );
        ensure!(
            idx < (BEATS_IN_MEASURE * rows_per_beat) as usize,
            "called `flush_measure` with more than one measure in buffer (rel_pos = {} out of max {})",
            rel_pos,
            BEATS_IN_MEASURE * rows_per_beat,
        );
        ensure!(
            note.key >= 0 && note.key < key_count,
            "note key {} outside range [0, {})",
            note.key,
            key_count
        );
        out_measure[idx * key_count as usize + note.key as usize] = note.kind as u8;
    }
    //Convert map into a string
    if measure_idx > 0 {
        //Add separator from previous measure
        write!(file, ",")?;
    }
    write!(file, "\n// Measure {}", measure_idx)?;
    for row in 0..(BEATS_IN_MEASURE * rows_per_beat) as usize {
        write!(file, "\n")?;
        for key in 0..key_count as usize {
            file.write_all(&[out_measure[row * key_count as usize + key]])?;
        }
    }
    Ok(())
}

fn write_notedata(file: &mut impl Write, sm: &Simfile) -> Result<()> {
    struct CurMeasure {
        first_note: usize,
        start_beat: BeatPos,
    }

    let key_count = sm.gamemode.key_count();
    let mut measure_counter = 0;
    let mut cur_measure = CurMeasure {
        first_note: 0,
        start_beat: BeatPos::from(0.),
    };
    for (note_idx, note) in sm.notes.iter().enumerate() {
        //Finish any pending measures
        while (note.beat - cur_measure.start_beat) >= BeatPos::from(BEATS_IN_MEASURE as f64) {
            write_measure(
                file,
                key_count,
                measure_counter,
                cur_measure.start_beat,
                &sm.notes[cur_measure.first_note..note_idx],
            )?;
            measure_counter += 1;
            cur_measure.first_note = note_idx;
            cur_measure.start_beat =
                cur_measure.start_beat + BeatPos::from(BEATS_IN_MEASURE as f64);
        }
    }
    //Finish the last pending measure
    write_measure(
        file,
        key_count,
        measure_counter,
        cur_measure.start_beat,
        &sm.notes[cur_measure.first_note..sm.notes.len()],
    )?;
    Ok(())
}

/// From the StepMania source,
/// [`GameManager.cpp`](https://github.com/stepmania/stepmania/blob/5_1-new/src/GameManager.cpp):
///
/// ```
/// // dance
/// { "dance-single",	4,	true,	StepsTypeCategory_Single },
/// { "dance-double",	8,	true,	StepsTypeCategory_Double },
/// { "dance-couple",	8,	true,	StepsTypeCategory_Couple },
/// { "dance-solo",		6,	true,	StepsTypeCategory_Single },
/// { "dance-threepanel",	3,	true,	StepsTypeCategory_Single }, // thanks to kurisu
/// { "dance-routine",	8,	false,	StepsTypeCategory_Routine },
/// // pump
/// { "pump-single",	5,	true,	StepsTypeCategory_Single },
/// { "pump-halfdouble",	6,	true,	StepsTypeCategory_Double },
/// { "pump-double",	10,	true,	StepsTypeCategory_Double },
/// { "pump-couple",	10,	true,	StepsTypeCategory_Couple },
/// // uh, dance-routine has that one bool as false... wtf? -aj
/// { "pump-routine",	10,	true,	StepsTypeCategory_Routine },
/// // kb7
/// { "kb7-single",		7,	true,	StepsTypeCategory_Single },
/// // { "kb7-small",		7,	true,	StepsTypeCategory_Single },
/// // ez2dancer
/// { "ez2-single",		5,	true,	StepsTypeCategory_Single },	// Single: TL,LHH,D,RHH,TR
/// { "ez2-double",		10,	true,	StepsTypeCategory_Double },	// Double: Single x2
/// { "ez2-real",		7,	true,	StepsTypeCategory_Single },	// Real: TL,LHH,LHL,D,RHL,RHH,TR
/// // parapara paradise
/// { "para-single",	5,	true,	StepsTypeCategory_Single },
/// // ds3ddx
/// { "ds3ddx-single",	8,	true,	StepsTypeCategory_Single },
/// // beatmania
/// { "bm-single5",		6,	true,	StepsTypeCategory_Single },	// called "bm" for backward compat
/// { "bm-versus5",		6,	true,	StepsTypeCategory_Single },	// called "bm" for backward compat
/// { "bm-double5",		12,	true,	StepsTypeCategory_Double },	// called "bm" for backward compat
/// { "bm-single7",		8,	true,	StepsTypeCategory_Single },	// called "bm" for backward compat
/// { "bm-versus7",		8,	true,	StepsTypeCategory_Single },	// called "bm" for backward compat
/// { "bm-double7",		16,	true,	StepsTypeCategory_Double },	// called "bm" for backward compat
/// // dance maniax
/// { "maniax-single",	4,	true,	StepsTypeCategory_Single },
/// { "maniax-double",	8,	true,	StepsTypeCategory_Double },
/// // technomotion
/// { "techno-single4",	4,	true,	StepsTypeCategory_Single },
/// { "techno-single5",	5,	true,	StepsTypeCategory_Single },
/// { "techno-single8",	8,	true,	StepsTypeCategory_Single },
/// { "techno-double4",	8,	true,	StepsTypeCategory_Double },
/// { "techno-double5",	10,	true,	StepsTypeCategory_Double },
/// { "techno-double8",	16,	true,	StepsTypeCategory_Double },
/// // pop'n music
/// { "pnm-five",		5,	true,	StepsTypeCategory_Single },	// called "pnm" for backward compat
/// { "pnm-nine",		9,	true,	StepsTypeCategory_Single },	// called "pnm" for backward compat
/// // cabinet lights and other fine StepsTypes that don't exist lol
/// { "lights-cabinet",	NUM_CabinetLight,	false,	StepsTypeCategory_Single }, // XXX disable lights autogen for now
/// // kickbox mania
/// { "kickbox-human", 4, true, StepsTypeCategory_Single },
/// { "kickbox-quadarm", 4, true, StepsTypeCategory_Single },
/// { "kickbox-insect", 6, true, StepsTypeCategory_Single },
/// { "kickbox-arachnid", 8, true, StepsTypeCategory_Single },
/// ```
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum Gamemode {
    DanceSingle,
    DanceDouble,
    DanceCouple,
    DanceSolo,
    DanceThreepanel,
    DanceRoutine,
    PumpSingle,
    PumpHalfdouble,
    PumpDouble,
    PumpCouple,
    PumpRoutine,
    Kb7Single,
    Ez2Single,
    Ez2Double,
    Ez2Real,
    ParaSingle,
    Ds3ddxSingle,
    BmSingle5,
    BmVersus5,
    BmDouble5,
    BmSingle7,
    BmVersus7,
    BmDouble7,
    ManiaxSingle,
    ManiaxDouble,
    TechnoSingle4,
    TechnoSingle5,
    TechnoSingle8,
    TechnoDouble4,
    TechnoDouble5,
    TechnoDouble8,
    PnmFive,
    PnmNine,
    KickboxHuman,
    KickboxQuadarm,
    KickboxInsect,
    KickboxArachnid,
}
impl Gamemode {
    pub fn key_count(&self) -> i32 {
        use Gamemode::*;
        match self {
            DanceSingle => 4,
            DanceDouble => 8,
            DanceCouple => 8,
            DanceSolo => 6,
            DanceThreepanel => 3,
            DanceRoutine => 8,
            PumpSingle => 5,
            PumpHalfdouble => 6,
            PumpDouble => 10,
            PumpCouple => 10,
            PumpRoutine => 10,
            Kb7Single => 7,
            Ez2Single => 5,
            Ez2Double => 10,
            Ez2Real => 7,
            ParaSingle => 5,
            Ds3ddxSingle => 8,
            BmSingle5 => 6,
            BmVersus5 => 6,
            BmDouble5 => 12,
            BmSingle7 => 8,
            BmVersus7 => 8,
            BmDouble7 => 16,
            ManiaxSingle => 4,
            ManiaxDouble => 8,
            TechnoSingle4 => 4,
            TechnoSingle5 => 5,
            TechnoSingle8 => 8,
            TechnoDouble4 => 8,
            TechnoDouble5 => 10,
            TechnoDouble8 => 16,
            PnmFive => 5,
            PnmNine => 9,
            KickboxHuman => 4,
            KickboxQuadarm => 4,
            KickboxInsect => 6,
            KickboxArachnid => 8,
        }
    }

    pub fn id(&self) -> &'static str {
        use Gamemode::*;
        match self {
            DanceSingle => "dance-single",
            DanceDouble => "dance-double",
            DanceCouple => "dance-couple",
            DanceSolo => "dance-solo",
            DanceThreepanel => "dance-threepanel",
            DanceRoutine => "dance-routine",
            PumpSingle => "pump-single",
            PumpHalfdouble => "pump-halfdouble",
            PumpDouble => "pump-double",
            PumpCouple => "pump-couple",
            PumpRoutine => "pump-routine",
            Kb7Single => "kb7-single",
            Ez2Single => "ez2-single",
            Ez2Double => "ez2-double",
            Ez2Real => "ez2-real",
            ParaSingle => "para-single",
            Ds3ddxSingle => "ds3ddx-single",
            BmSingle5 => "bm-single5",
            BmVersus5 => "bm-versus5",
            BmDouble5 => "bm-double5",
            BmSingle7 => "bm-single7",
            BmVersus7 => "bm-versus7",
            BmDouble7 => "bm-double7",
            ManiaxSingle => "maniax-single",
            ManiaxDouble => "maniax-double",
            TechnoSingle4 => "techno-single4",
            TechnoSingle5 => "techno-single5",
            TechnoSingle8 => "techno-single8",
            TechnoDouble4 => "techno-double4",
            TechnoDouble5 => "techno-double5",
            TechnoDouble8 => "techno-double8",
            PnmFive => "pnm-five",
            PnmNine => "pnm-nine",
            KickboxHuman => "kickbox-human",
            KickboxQuadarm => "kickbox-quadarm",
            KickboxInsect => "kickbox-insect",
            KickboxArachnid => "kickbox-arachnid",
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Difficulty {
    Beginner,
    Easy,
    Medium,
    Hard,
    Challenge,
    Edit,
}
impl Difficulty {
    fn name(&self) -> &'static str {
        use Difficulty::*;
        match self {
            Beginner => "Beginner",
            Easy => "Easy",
            Medium => "Medium",
            Hard => "Hard",
            Challenge => "Challenge",
            Edit => "Edit",
        }
    }
}

/// Represents an absolute position in beats, where 0 is the first beat of the song.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct BeatPos {
    frac: i32,
}
impl BeatPos {
    const FIXED_POINT: i32 = 48;
    pub const EPSILON: BeatPos = BeatPos { frac: 1 };

    /// Get the beat number as an `f64`.
    pub fn as_num(self) -> f64 {
        self.into()
    }

    /// Round this beat position to the nearest 1/`divisions` of a beat.
    pub fn round(mut self, divisions: i32) -> Self {
        let round_to = Self::FIXED_POINT / divisions;
        self.frac += round_to / 2;
        self.frac -= self.frac % round_to;
        self
    }

    /// Get the denominator of the most-simplified version of this beat (eg. 1/2, 3/4, 0/1, 19/16).
    pub fn denominator(self) -> i32 {
        let mut num = self.frac;
        let mut den = BeatPos::FIXED_POINT;
        for &factor in [2, 3].iter() {
            while num % factor == 0 && den % factor == 0 {
                num /= factor;
                den /= factor;
            }
        }
        den
    }
}
impl From<f64> for BeatPos {
    fn from(float: f64) -> BeatPos {
        Self {
            frac: (float * Self::FIXED_POINT as f64).round() as i32,
        }
    }
}
impl From<BeatPos> for f64 {
    fn from(beat: BeatPos) -> f64 {
        beat.frac as f64 / BeatPos::FIXED_POINT as f64
    }
}
impl ops::AddAssign for BeatPos {
    fn add_assign(&mut self, rhs: Self) {
        self.frac += rhs.frac;
    }
}
impl ops::Add for BeatPos {
    type Output = Self;
    fn add(mut self, rhs: Self) -> Self {
        self += rhs;
        self
    }
}
impl ops::SubAssign for BeatPos {
    fn sub_assign(&mut self, rhs: Self) {
        self.frac -= rhs.frac;
    }
}
impl ops::Sub for BeatPos {
    type Output = Self;
    fn sub(mut self, rhs: Self) -> Self {
        self -= rhs;
        self
    }
}
impl fmt::Display for BeatPos {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.as_num())
    }
}

#[derive(Debug, Clone)]
pub struct Note {
    pub kind: char,
    pub beat: BeatPos,
    pub key: i32,
}
impl Note {
    pub const KIND_HIT: char = '1';
    pub const KIND_HEAD: char = '2';
    pub const KIND_TAIL: char = '3';

    pub fn is_hit(&self) -> bool {
        self.kind == Self::KIND_HIT
    }

    pub fn is_head(&self) -> bool {
        self.kind == Self::KIND_HEAD
    }

    pub fn is_tail(&self) -> bool {
        self.kind == Self::KIND_TAIL
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ControlPoint {
    /// First beat of the control point.
    pub beat: BeatPos,
    /// Length of a beat in seconds.
    pub beat_len: f64,
}
impl ControlPoint {
    pub fn bpm(&self) -> f64 {
        60. / self.beat_len
    }
}

#[derive(Debug, Clone)]
pub struct ToTime<'a> {
    bpms: &'a [ControlPoint],
    cur_idx: usize,
    cur_time: f64,
}
impl ToTime<'_> {
    pub fn new(sm: &Simfile) -> ToTime {
        ToTime {
            bpms: &sm.bpms,
            cur_idx: 0,
            cur_time: -sm.offset,
        }
    }

    /// Returns incorrect results if called with non-monotonic beat positions.
    /// If needing to seek back in time, create a new `ToTime` or make "checkpoints" with `Clone`.
    pub fn beat_to_time(&mut self, beat: BeatPos) -> f64 {
        //Advance control points
        while self.cur_idx + 1 < self.bpms.len() {
            let cur_bpm = &self.bpms[self.cur_idx];
            let next_bpm = &self.bpms[self.cur_idx + 1];
            if beat >= next_bpm.beat {
                //Advance to this control point
                let adv_time = (next_bpm.beat - cur_bpm.beat).as_num() * cur_bpm.beat_len;
                self.cur_time += adv_time;
                self.cur_idx += 1;
            } else {
                //Still within the current timing point
                break;
            }
        }
        //Use the current control point to determine the time corresponding to this beat
        let cur_bpm = &self.bpms[self.cur_idx];
        self.cur_time + (beat - cur_bpm.beat).as_num() * cur_bpm.beat_len
    }
}
