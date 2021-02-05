use std::collections::BTreeSet;
use std::fmt;
use std::fs::{read, rename, File};
use std::io::{BufWriter, Write};
use std::mem::drop;
use std::path::{Path, PathBuf};

use anyhow::{Context, Error, Result};
use clap::{App, Arg};
use glob::glob;

const BUF_SIZE: usize = 1 << 20;

fn main() -> Result<()> {
    let matches = App::new("fcrlf")
        .about("Converter of file's CRLF line delimiter.")
        .arg(
            Arg::with_name("crlf")
                .short("w")
                .long("crlf")
                .conflicts_with_all(&["lf", "cr"])
                .help("Convert to CRLF"),
        )
        .arg(
            Arg::with_name("lf")
                .short("u")
                .long("lf")
                .conflicts_with_all(&["crlf", "cr"])
                .help("Convert to LF"),
        )
        .arg(
            Arg::with_name("cr")
                .short("m")
                .long("cr")
                .conflicts_with_all(&["lf", "crlf"])
                .help("Convert to CR"),
        )
        .arg(
            Arg::with_name("detect")
                .short("d")
                .long("detect")
                .help("Detect only and don't perform conversion"),
        )
        .arg(
            Arg::with_name("patterns")
                .required(true)
                .multiple(true)
                .help("Files to convert"),
        )
        .get_matches();

    let do_covert = !matches.is_present("detect");
    let target_delim = match () {
        () if matches.is_present("crlf") => Delim::CRLF,
        () if matches.is_present("lf") => Delim::LF,
        () if matches.is_present("cr") => Delim::CR,
        _ => {
            return Err(Error::msg(
                "No target delimiter is specified, use '--lf', '--crlf' or '--cr'.",
            ))
        }
    };
    let mut target_delim_set = BTreeSet::new();
    target_delim_set.insert(target_delim);
    let target_delim_set = target_delim_set;

    let patterns = matches.values_of("patterns").expect("files should exists");

    for pat in patterns {
        let pathes = glob(pat).with_context(|| format!("listing files for pattern: {:?}", pat))?;
        for p in pathes {
            let p = p.with_context(|| format!("reading path in {:?}", pat))?;
            if !p.exists() || !p.is_file() {
                continue;
            }

            let file_contents_raw =
                read(&p).with_context(|| format!("reading file contents of {}", PathFmt(&p)))?;

            let file_contents = FileContents::from_bytes(&file_contents_raw);

            let delim_types = file_contents.delim_types();
            if !delim_types.is_subset(&target_delim_set) {
                if do_covert {
                    let tmp_path = tmp_path(&p);
                    let f = File::create(&tmp_path).with_context(|| {
                        format!("creating tmporary file: {}", PathFmt(&tmp_path))
                    })?;
                    let mut f = BufWriter::with_capacity(BUF_SIZE, f);

                    file_contents
                        .write_to(&mut f, target_delim)
                        .with_context(|| {
                            format!(
                                "writing file contents to tmporary file: {}",
                                PathFmt(&tmp_path)
                            )
                        })?;

                    f.flush().with_context(|| {
                        format!(
                            "writing file contents to tmporary file: {}",
                            PathFmt(&tmp_path)
                        )
                    })?;
                    drop(f);

                    rename(&tmp_path, &p).with_context(|| {
                        format!(
                            "renaming temporary file: {} => {}",
                            PathFmt(&tmp_path),
                            PathFmt(&p)
                        )
                    })?;
                } else {
                    println!("{}: {}", PathFmt(&p), DelimSetFmt(&delim_types));
                }
            }
        }
    }

    Ok(())
}

fn tmp_path(path: &Path) -> PathBuf {
    assert!(path.is_file(), "argument should be file: {:?}", path);

    let file_name = path
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or(String::new());

    for i in 0u64.. {
        let file_name = format!("{}.tmp{}", file_name, i);
        let res = path.with_file_name(file_name);

        if !res.exists() {
            return res;
        }
    }
    unreachable!()
}

#[derive(Debug, PartialEq)]
struct FileContents {
    lines: Vec<Line>,
}

impl FileContents {
    fn from_bytes(mut bytes: &[u8]) -> FileContents {
        let mut lines = Vec::new();
        let mut cur_line = Line::new();

        while !bytes.is_empty() {
            if bytes.starts_with(b"\r\n") {
                cur_line.line_end = Some(Delim::CRLF);
                bytes = &bytes[2..];
                lines.push(cur_line);
                cur_line = Line::new();
            } else if bytes[0] == b'\n' {
                cur_line.line_end = Some(Delim::LF);
                bytes = &bytes[1..];
                lines.push(cur_line);
                cur_line = Line::new();
            } else if bytes[0] == b'\r' {
                cur_line.line_end = Some(Delim::CR);
                bytes = &bytes[1..];
                lines.push(cur_line);
                cur_line = Line::new();
            } else {
                cur_line.text.push(bytes[0]);
                bytes = &bytes[1..];
            }
        }

        lines.push(cur_line);
        FileContents { lines }
    }

    fn delim_types(&self) -> BTreeSet<Delim> {
        let mut types = BTreeSet::new();

        for l in &self.lines {
            if let Some(d) = l.line_end {
                types.insert(d);
            }
        }

        types
    }

    fn write_to(&self, w: &mut impl Write, delim: Delim) -> Result<()> {
        for l in &self.lines {
            l.write_to(w, delim)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq)]
struct Line {
    text: Vec<u8>,
    line_end: Option<Delim>,
}

impl Line {
    fn new() -> Line {
        Line {
            text: Vec::new(),
            line_end: None,
        }
    }

    fn write_to(&self, w: &mut impl Write, delim: Delim) -> Result<()> {
        w.write_all(&self.text)?;
        if let Some(_) = self.line_end {
            delim.write_to(w)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum Delim {
    LF,
    CR,
    CRLF,
}

impl Delim {
    fn write_to(self, w: &mut impl Write) -> Result<()> {
        match self {
            Delim::LF => w.write_all(b"\n")?,
            Delim::CR => w.write_all(b"\r")?,
            Delim::CRLF => w.write_all(b"\r\n")?,
        }
        Ok(())
    }
}

impl fmt::Display for Delim {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Delim::LF => write!(f, "LF"),
            Delim::CR => write!(f, "CR"),
            Delim::CRLF => write!(f, "CRLF"),
        }
    }
}

#[derive(Debug)]
struct DelimSetFmt<'a>(&'a BTreeSet<Delim>);

impl<'a> fmt::Display for DelimSetFmt<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.0.is_empty() {
            write!(f, "NO_DELIM")?;
        } else {
            for (i, v) in self.0.iter().enumerate() {
                if i == 0 {
                    write!(f, "{}", v)?;
                } else {
                    write!(f, ", {}", v)?;
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_file() {
        let raw = b"abc\r\ndef\nghi\rj";
        let parsed = FileContents::from_bytes(raw);

        assert_eq!(
            parsed,
            FileContents {
                lines: vec![
                    Line {
                        text: b"abc".to_vec(),
                        line_end: Some(Delim::CRLF)
                    },
                    Line {
                        text: b"def".to_vec(),
                        line_end: Some(Delim::LF)
                    },
                    Line {
                        text: b"ghi".to_vec(),
                        line_end: Some(Delim::CR)
                    },
                    Line {
                        text: b"j".to_vec(),
                        line_end: None
                    },
                ]
            }
        );
    }

    #[test]
    fn parse_write_round_trip() {
        let raw = b"abc\r\ndef\nghi\rj";
        let parsed = FileContents::from_bytes(raw);
        let mut written = Vec::<u8>::new();
        parsed.write_to(&mut written, Delim::CRLF).unwrap();
        assert_eq!(&written, b"abc\r\ndef\r\nghi\r\nj");
    }
}

#[derive(Debug)]
struct PathFmt<'a>(&'a Path);

impl<'a> fmt::Display for PathFmt<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(s) = self.0.to_str() {
            write!(f, "{}", s)
        } else {
            write!(f, "{:?}", self.0)
        }
    }
}
