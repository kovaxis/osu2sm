use crate::prelude::*;

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
    {radar0}, {radar1}, {radar2}, {radar3}, {radar4}:
{notedata};
"#,
                gamemode = chart.gamemode,
                desc = chart.desc,
                diff_name = chart.diff_name,
                diff_num = chart.diff_num,
                radar0 = chart.radar[0],
                radar1 = chart.radar[1],
                radar2 = chart.radar[2],
                radar3 = chart.radar[3],
                radar4 = chart.radar[4],
                notedata = chart.measures,
            )?;
        }
        Ok(())
    }
}

#[derive(Debug, Default, Clone)]
pub(crate) struct Chart {
    pub gamemode: String,
    pub desc: String,
    pub diff_name: String,
    pub diff_num: f64,
    pub radar: [f64; 5],
    pub measures: String,
}
