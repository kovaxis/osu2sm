use crate::prelude::*;

const BEATS_IN_MEASURE: i32 = 4;
const AVAILABLE_DIFFICULTIES: &[Difficulty] = &[
    Difficulty::Beginner,
    Difficulty::Easy,
    Difficulty::Medium,
    Difficulty::Hard,
    Difficulty::Challenge,
    Difficulty::Edit,
];

#[derive(Debug, Default, Clone)]
pub(crate) struct Simfile {
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
    pub bpms: Vec<(f64, f64)>,
    pub stops: Vec<(f64, f64)>,
    pub sample_start: Option<f64>,
    pub sample_len: Option<f64>,
    pub charts: Vec<Chart>,
}
impl Simfile {
    pub(crate) fn save(&self, path: &Path) -> Result<()> {
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
            title = self.title,
            subtitle = self.subtitle,
            artist = self.artist,
            title_t = self.title_trans,
            subtitle_t = self.subtitle_trans,
            artist_t = self.artist_trans,
            genre = self.genre,
            credit = self.credit,
            banner = as_utf8(&self.banner, "BANNER")?,
            bg = as_utf8(&self.background, "BACKGROUND")?,
            lyrics = as_utf8(&self.lyrics, "LYRICSPATH")?,
            cdtitle = as_utf8(&self.cdtitle, "CDTITLE")?,
            music = as_utf8(&self.music, "MUSIC")?,
            offset = self.offset,
            sample_start = self
                .sample_start
                .map(|s| format!("{}", s))
                .unwrap_or_else(String::new),
            sample_len = self
                .sample_len
                .map(|l| format!("{}", l))
                .unwrap_or_else(String::new),
            bpms = {
                let mut bpms = String::new();
                let mut first = true;
                for (beat, bpm) in self.bpms.iter() {
                    if first {
                        first = false;
                    } else {
                        bpms.push(',');
                    }
                    write!(bpms, "{}={}", beat, bpm).unwrap();
                }
                bpms
            },
        )?;
        for chart in self.charts.iter() {
            write!(
                file,
                r#"
#NOTES:
    {gamemode}:
    {desc}:
    {diff_name}:
    {diff_num}:
    {radar0}, {radar1}, {radar2}, {radar3}, {radar4}:"#,
                gamemode = chart.gamemode.id(),
                desc = chart.desc,
                diff_name = chart.difficulty.name(),
                diff_num = chart.difficulty_num,
                radar0 = chart.radar[0],
                radar1 = chart.radar[1],
                radar2 = chart.radar[2],
                radar3 = chart.radar[3],
                radar4 = chart.radar[4],
            )?;
            write_notedata(&mut file, &chart)?;
            write!(file, ";")?;
        }
        Ok(())
    }

    pub(crate) fn file_deps(&self) -> impl Iterator<Item = &Path> {
        self.banner
            .as_deref()
            .into_iter()
            .chain(self.background.as_deref().into_iter())
            .chain(self.lyrics.as_deref().into_iter())
            .chain(self.cdtitle.as_deref().into_iter())
            .chain(self.music.as_deref().into_iter())
    }

    /// There seems to be a max of 6 difficulties, so use them wisely and sort them.
    pub(crate) fn spread_difficulties(&mut self) -> Result<()> {
        //I'm not so sure about how robust is this algorithm, so I'll leave this here just in case
        let debug = false;
        if debug {
            println!("  spreading difficulties");
        }
        //Create an auxiliary vec holding chart indices and difficulties
        let mut order = self
            .charts
            .iter()
            .enumerate()
            .map(|(idx, chart)| (idx, self.difficulty_of(chart)))
            .collect::<Vec<_>>();
        if debug {
            println!("    raw difficulties: {:?}", order);
        }
        //Sort by difficulty
        order.sort_by_key(|(_, d)| SortableFloat(*d));
        if debug {
            println!("    sorted difficulties: {:?}", order);
        }
        //Remove difficulties, mantaining as much spread as possible
        while order.len() > AVAILABLE_DIFFICULTIES.len() {
            //Find the smallest gap
            let (mut smallest, _) = order
                .windows(2)
                .enumerate()
                .min_by_key(|(_idx, window)| SortableFloat(window[1].1 - window[0].1))
                .unwrap();
            let get_gap_before = |idx: usize| {
                if idx <= 0 || idx >= order.len() {
                    99999.
                } else {
                    order[idx].1 - order[idx - 1].1
                }
            };
            if get_gap_before(smallest) > get_gap_before(smallest + 2) {
                smallest += 1;
            }
            //Remove this chart :(
            order.remove(smallest);
        }
        if debug {
            println!("    with conflicts resolved: {:?}", order);
        }
        //Reorder charts
        for (idx, diff) in order.iter() {
            self.charts[*idx].difficulty_num = *diff;
        }
        self.charts
            .sort_by_key(|chart| SortableFloat(chart.difficulty_num));
        if debug {
            println!(
                "    final chart difficulties: {:?}",
                self.charts
                    .iter()
                    .map(|chart| chart.difficulty_num)
                    .collect::<Vec<_>>()
            );
        }
        //Reassign difficulty names from numbers
        let mut difficulties = self
            .charts
            .iter()
            .map(|chart| {
                let (diff, _d) = AVAILABLE_DIFFICULTIES
                    .iter()
                    .enumerate()
                    .min_by_key(|(_i, diff)| {
                        SortableFloat((diff.numeric() - chart.difficulty_num).abs())
                    })
                    .unwrap();
                diff as isize
            })
            .collect::<Vec<_>>();
        if debug {
            println!("    diff indices: {:?}", difficulties);
        }
        //Resolve conflicts
        loop {
            let mut conflict = None;
            for (i, window) in difficulties.windows(2).enumerate() {
                if window[1] == window[0] {
                    //Conflict
                    //See which way is the conflict solved faster
                    let direction_cost = |idx: usize, dir: isize| {
                        let mut idx = idx as isize;
                        let mut occupied_if = difficulties[idx as usize];
                        let mut cost = 0.;
                        while occupied_if >= 0
                            && occupied_if < AVAILABLE_DIFFICULTIES.len() as isize
                            && idx >= 0
                            && idx < difficulties.len() as isize
                        {
                            if (difficulties[idx as usize] - occupied_if) * dir <= 0 {
                                idx += dir;
                                occupied_if += dir;
                                cost += 1.;
                            } else {
                                break;
                            }
                        }
                        if occupied_if < 0 || occupied_if >= AVAILABLE_DIFFICULTIES.len() as isize {
                            //Saturated. Max cost
                            9999.
                        } else {
                            cost
                        }
                    };
                    if debug {
                        println!("    conflict on {} - {}", i, i + 1);
                    }
                    if direction_cost(i, -1) < direction_cost(i + 1, 1) {
                        //Solve to the left
                        conflict = Some((i, -1));
                    } else {
                        //Solve to the right
                        conflict = Some((i + 1, 1));
                    }
                    break;
                }
            }
            match conflict {
                Some((idx, dir)) => {
                    let mut idx = idx as isize;
                    if debug {
                        println!("      solving on idx {}, direction {}", idx, dir);
                    }
                    let mut set_to = difficulties[idx as usize] + dir;
                    while idx >= 0
                        && idx < difficulties.len() as isize
                        && (difficulties[idx as usize] - set_to) * dir <= 0
                    {
                        set_to = set_to.min(AVAILABLE_DIFFICULTIES.len() as isize - 1).max(0);
                        if debug {
                            println!(
                                "      moving difficulties[{}] == {} -> {}",
                                idx, difficulties[idx as usize], set_to
                            );
                        }
                        difficulties[idx as usize] = set_to;
                        idx += dir;
                        set_to += dir;
                    }
                }
                None => break,
            }
        }
        if debug {
            println!(
                "    diff indices with conflicts resolved: {:?}",
                difficulties
            );
        }
        //Convert back from difficulty indices to actual difficulties
        for (chart, diff_idx) in self.charts.iter_mut().zip(difficulties) {
            chart.difficulty = AVAILABLE_DIFFICULTIES[diff_idx as usize];
            chart.difficulty_num = chart.difficulty_num.round();
        }
        if debug {
            println!(
                "    final chart difficulties: {:?}",
                self.charts
                    .iter()
                    .map(|chart| format!("{} ({})", chart.difficulty.name(), chart.difficulty_num))
                    .collect::<Vec<_>>()
            );
        }
        Ok(())
    }

    /// Get the estimated difficulty of a certain chart.
    pub(crate) fn difficulty_of(&self, chart: &Chart) -> f64 {
        fn adapt_range(src: (f64, f64), dst: (f64, f64), val: f64) -> f64 {
            dst.0 + (val - src.0) / (src.1 - src.0) * (dst.1 - dst.0)
        }
        let diff = adapt_range((6., 14.), (1., 12.), (chart.notes.len() as f64).log2());
        diff.max(1.)
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
                rel_pos >= BeatPos::from_float(0.),
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
        out_measure[idx * key_count as usize + note.key as usize] = b'1';
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

fn write_notedata(file: &mut impl Write, chart: &Chart) -> Result<()> {
    struct CurMeasure {
        first_note: usize,
        start_beat: BeatPos,
    }

    let key_count = chart.gamemode.key_count();
    let mut measure_counter = 0;
    let mut cur_measure = CurMeasure {
        first_note: 0,
        start_beat: BeatPos::from_float(0.),
    };
    for (note_idx, note) in chart.notes.iter().enumerate() {
        //Finish any pending measures
        while (note.beat - cur_measure.start_beat) >= BeatPos::from_float(BEATS_IN_MEASURE as f64) {
            write_measure(
                file,
                key_count,
                measure_counter,
                cur_measure.start_beat,
                &chart.notes[cur_measure.first_note..note_idx],
            )?;
            measure_counter += 1;
            cur_measure.first_note = note_idx;
            cur_measure.start_beat =
                cur_measure.start_beat + BeatPos::from_float(BEATS_IN_MEASURE as f64);
        }
    }
    //Finish the last pending measure
    write_measure(
        file,
        key_count,
        measure_counter,
        cur_measure.start_beat,
        &chart.notes[cur_measure.first_note..chart.notes.len()],
    )?;
    Ok(())
}

#[derive(Debug, Clone)]
pub(crate) struct Chart {
    pub gamemode: Gamemode,
    pub desc: String,
    pub difficulty: Difficulty,
    pub difficulty_num: f64,
    pub radar: [f64; 5],
    pub notes: Vec<Note>,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum Gamemode {
    DanceSingle,
}
impl Gamemode {
    pub fn key_count(&self) -> i32 {
        use Gamemode::*;
        match self {
            DanceSingle => 4,
        }
    }

    pub fn id(&self) -> &'static str {
        match self {
            Gamemode::DanceSingle => "dance-single",
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum Difficulty {
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

    fn numeric(&self) -> f64 {
        use Difficulty::*;
        match self {
            Beginner => 1.,
            Easy => 2.,
            Medium => 3.5,
            Hard => 5.,
            Challenge => 6.5,
            Edit => 8.,
        }
    }
}

/// Represents an absolute position in beats, where 0 is the first beat of the song.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct BeatPos {
    frac: i32,
}
impl BeatPos {
    const FIXED_POINT: i32 = 48;

    pub fn from_float(float: f64) -> BeatPos {
        Self {
            frac: (float * Self::FIXED_POINT as f64).round() as i32,
        }
    }

    pub fn as_float(&self) -> f64 {
        self.frac as f64 / Self::FIXED_POINT as f64
    }
}
impl ops::Add for BeatPos {
    type Output = Self;
    fn add(mut self, rhs: Self) -> Self {
        self.frac += rhs.frac;
        self
    }
}
impl ops::Sub for BeatPos {
    type Output = Self;
    fn sub(mut self, rhs: Self) -> Self {
        self.frac -= rhs.frac;
        self
    }
}
impl fmt::Display for BeatPos {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "BeatPos({} beats)", self.as_float())
    }
}

#[derive(Debug, Clone)]
pub(crate) struct Note {
    pub beat: BeatPos,
    pub key: i32,
}
