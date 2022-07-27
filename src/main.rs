use chrono::{DateTime, Datelike, NaiveDateTime, NaiveDate, NaiveTime};
use exif::{In, Tag};
use human_bytes::human_bytes;
use regex::Regex;
use std::collections::HashSet;
use std::env;
use std::ffi::OsStr;
use std::fs::{self, DirEntry};
use std::io;
use std::path::{Path, PathBuf};
use std::process::exit;
use std::time::UNIX_EPOCH;

fn main() {
    let mut args: Vec<String> = env::args().collect();
    args.remove(0);

    let mut mode = Mode::Move;
    let mut mode_set = false;
    let mut verbose: bool = false;
    let mut source_folder = Option::None;
    let mut target_folder = Option::None;
    let mut skip_read_next_value = false;
    for (i, arg) in args.iter().enumerate() {
        if skip_read_next_value {
            skip_read_next_value = false;
            continue;
        }
        if arg == "--verbose" || arg == "-v" {
            verbose = true;
        } else if arg == "--dry-run" || arg == "-d" {
            if mode_set {
                exit_with_message::<bool>("Only one mode can be chosen.");
            }
            mode = Mode::DryRun;
            mode_set = true;
        } else if arg == "--copy" || arg == "-c" {
            if mode_set {
                exit_with_message::<bool>("Only one mode can be chosen.");
            }
            mode = Mode::Copy;
            mode_set = true;
        } else if arg == "--move" || arg == "-m" {
            if mode_set {
                exit_with_message::<bool>("Only one mode can be chosen.");
            }
            mode = Mode::Move;
            mode_set = true;
        } else if arg == "--target" || arg == "-t" {
            target_folder = args.get(i + 1);
            skip_read_next_value = true;
        } else {
            if source_folder.is_none() {
                source_folder = Option::Some(arg.to_owned());
            } else {
                exit_with_message::<bool>("Too many arguments given.");
            }
        }
    }

    let source_directory = Path::new(source_folder.get_or_insert(".".to_string()));
    let target_directory = target_folder
        .map(|s| Path::new(s))
        .unwrap_or_else(|| exit_with_message("No target folder supplied."));
    if !target_directory.exists() {
        exit_with_message::<bool>(
            "Target folder does not exists or you are missing the required permissions.",
        );
    }

    let mut target_parents = HashSet::new();
    let date_regex = Regex::new(r"(20[012]\d[01]\d\d{2})").unwrap();

    visit_dirs(
        &source_directory,
        &mut handle_file(
            verbose,
            target_directory,
            &mode,
            &mut target_parents,
            &date_regex,
        ),
    )
    .unwrap();
}

fn exit_with_message<T>(message: &str) -> T {
    println!("{}", message);
    exit(1);
}

fn visit_dirs(dir: &Path, cb: &mut dyn FnMut(&DirEntry)) -> io::Result<()> {
    if dir.is_dir() {
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                visit_dirs(&path, cb)?;
            } else {
                cb(&entry);
            }
        }
    }
    Ok(())
}

fn handle_file<'a>(
    verbose: bool,
    target_directory: &'a Path,
    mode: &'a Mode,
    target_parents: &'a mut HashSet<PathBuf>,
    date_regex: &'a Regex,
) -> impl FnMut(&DirEntry) + 'a {
    move |dir_entry: &DirEntry| -> () {
        let source_path = dir_entry.path();

        if is_supported_file_type(&source_path) {
            match handle_image(
                verbose,
                &source_path,
                target_directory,
                mode,
                target_parents,
                date_regex,
            ) {
                Ok(target_file) => {
                    let parent = target_file
                        .parent()
                        .expect("File and parent exist")
                        .to_owned()
                        .clone();
                    if !target_parents.contains(&parent) {
                        target_parents.insert(parent);
                    }
                }
                Err(e) => {
                    println!("Error in {:?}: {}", source_path, e);
                }
            }
        } else {
            if verbose {
                println!("=========");
                println!("File {:?} is not a supported file type", source_path);
            }
        }
    }
}

fn is_supported_file_type(source_path: &PathBuf) -> bool {
    let is_supported = source_path
        .extension()
        .and_then(OsStr::to_str)
        .filter(|&e| ["png", "jpg", "jpeg", "tif", "mp4"].contains(&e.to_lowercase().as_str()))
        .is_some();
    is_supported
}

fn handle_image(
    verbose: bool,
    source_path: &PathBuf,
    target_directory: &Path,
    mode: &Mode,
    target_parents: &HashSet<PathBuf>,
    date_regex: &Regex,
) -> Result<PathBuf, String> {
    println!("---------------");
    if verbose {
        println!("Found file {:?}.", source_path);
    }
    let date_time = extract_date_time(&source_path, date_regex)?;
    if verbose {
        println!(
            "Image {:?} was taken at DateTime {}",
            source_path, date_time
        )
    }
    let mut target_path = target_directory
        .join(date_time.year().to_string())
        .join(date_time.month().to_string())
        .join(
            source_path
                .file_name()
                .expect("we only supply valid files."),
        );

    if target_path.exists() {
        match handle_file_exists_at_target(&source_path, &target_path) {
            Some(path_resolution) => target_path = path_resolution,
            None => return Ok(target_path),
        }
    }
    match mode {
        Mode::DryRun => println!(
            "Dry run: Copy/Move source file {:?} to target {:?}",
            source_path, target_path
        ),
        Mode::Move => {
            handle_missing_parents(verbose, &target_path, target_parents)?;
            println!(
                "Moving source file {:?} to target {:?}",
                source_path, target_path
            );
            fs::rename(&source_path, &target_path).map_err(|e| e.to_string())?;
        }
        Mode::Copy => {
            handle_missing_parents(verbose, &target_path, target_parents)?;
            println!(
                "Copying source file {:?} to target {:?}",
                source_path, target_path
            );
            fs::copy(&source_path, &target_path).map_err(|e| e.to_string())?;
        }
    }
    Ok(target_path)
}

fn handle_missing_parents<'a>(
    verbose: bool,
    target_path: &'a PathBuf,
    target_parents: &HashSet<PathBuf>,
) -> Result<(), String> {
    let parent = target_path.parent().expect("is valid.");
    Ok(if !target_parents.contains(&parent.to_path_buf()) {
        if verbose {
            println!("Creating folder {:?}", parent);
        }
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    })
}

fn extract_date_time(path: &PathBuf, date_regex: &Regex) -> Result<NaiveDateTime, String> {
    let result_from_media_metadata = if is_image(path) {
        let exifreader = exif::Reader::new();
        std::fs::File::open(path)
            .map_err(|e| e.to_string())
            .map(|inner| std::io::BufReader::new(inner))
            .and_then(|mut inner| {
                exifreader
                    .read_from_container(&mut inner)
                    .map_err(|e| e.to_string())
            })
            .and_then(|inner| {
                inner
                    .get_field(Tag::DateTimeOriginal, In::PRIMARY)
                    .or(inner.get_field(Tag::DateTime, In::PRIMARY))
                    .or(inner.get_field(Tag::DateTimeDigitized, In::PRIMARY))
                    .map(|field| field.display_value().to_string())
                    .ok_or("DateTime tag is missing.".to_string())
            })
            .and_then(|inner| {
                NaiveDateTime::parse_from_str(inner.as_str().trim(), "%Y-%m-%d %H:%M:%S")
                    .map_err(|e| e.to_string())
            })
    } else {
        ffprobe::ffprobe(path)
            .map_err(|e| e.to_string())?
            .format
            .tags
            .ok_or("Can't rad mp4 creation date time.".to_string())
            .and_then(|tag| {
                tag.creation_time
                    .ok_or("Can't read mp4 creation date time.".to_string())
            })
            .and_then(|str| DateTime::parse_from_rfc3339(str.as_str()).map_err(|e| e.to_string()))
            .map(|date_time| date_time.naive_local())
    };

    result_from_media_metadata
        .ok()
        .or_else(extract_media_creation_time_from_filename(date_regex, path))
        .or_else(extract_media_creation_time_from_file_metadata(path))
        .ok_or("Could not determine a media file creation date.".to_owned())
}

fn extract_media_creation_time_from_filename<'a>(
    date_regex: &'a Regex,
    path: &'a PathBuf,
) -> impl FnOnce() -> Option<NaiveDateTime> + 'a {
    || {
        date_regex
            .captures(path.file_name().map(|s| s.to_str()).unwrap().unwrap())
            .filter(|c| c.len() == 1)
            .map(|c| c.get(0).unwrap().as_str())
            .and_then(|s| {
                NaiveDateTime::parse_from_str(
                    (s.to_owned() + " 0:0:0").as_str().trim(),
                    "%Y%m%d %H:%M:%S",
                )
                .ok()
            })
    }
}

fn extract_media_creation_time_from_file_metadata<'a>(
    path: &'a PathBuf,
) -> impl FnOnce() -> Option<NaiveDateTime> + 'a {
    move || {
        let file_creation_date = path
            .metadata()
            .and_then(|m| m.created())
            .ok()
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .and_then(|duration| NaiveDateTime::from_timestamp_opt(duration.as_secs() as i64, 0));

        match file_creation_date {
            Some(date) => {
                println!(
                    "Could not determine creation time of media file {:?}",
                    &path
                );
                println!("Choose a resolution:");
                println!("1) Use the file creation time: {:?}", date);
                println!("2) Enter year and month manually.");
                let answer = loop {
                    let mut input = String::new();
                    _ = std::io::stdin().read_line(&mut input);
                    input = input.trim().to_string();
                    if ["1", "2"].contains(&input.as_str()) {
                        println!("Your option: {}", input);
                        break input;
                    } else {
                        println!("Invalid option {}. Choose 1, 2", input)
                    }
                };
                if "1" == answer {
                    return Some(date);
                } else if "2" == answer {
                    println!("Enter the year as number, e.g. 2022");
                    let year = loop {
                        let mut input = String::new();
                        _ = std::io::stdin().read_line(&mut input);
                        input = input.trim().to_string();
                        if input.len() != 4 {
                            println!("Invalid input {}. expected a 4 digit number, e.g. 2022", input);
                            continue;
                        }
                        if let Ok(year) = input.parse::<i32>() {
                            println!("Your option: {}", input);
                            break year;
                        } else {
                            println!("Invalid input {}. expected a number, e.g. 2022", input)
                        }
                    };
                    println!("Enter the month as number, e.g. 12");
                    let month = loop {
                        let mut input = String::new();
                        _ = std::io::stdin().read_line(&mut input);
                        input = input.trim().to_string();
                        if input.len() != 2 {
                            println!("Invalid input {}. expected a two digit number, e.g. 12", input);
                            continue;
                        }
                        if let Ok(month) = input.parse::<u32>() {
                            println!("Your option: {}", input);
                            break month;
                        } else {
                            println!("Invalid input {}. expected a number, e.g. 12", input)
                        }
                    };
                    return Some(NaiveDateTime::new(NaiveDate::from_ymd(year, month, 1), NaiveTime::from_hms(0,0,0)));
                } else {
                    panic!("Unreachable.")
                }
            }
            None => None,
        }
    }
}

fn is_image(path: &PathBuf) -> bool {
    path.extension()
        .and_then(OsStr::to_str)
        .filter(|&e| ["png", "jpg", "jpeg", "tif"].contains(&e.to_lowercase().as_str()))
        .is_some()
}

fn handle_file_exists_at_target(source_path: &PathBuf, target_path: &PathBuf) -> Option<PathBuf> {
    println!("Filename collision detected.");
    println!(
        "The file {:?} already exists at target {:?}",
        source_path, target_path
    );
    if source_path.metadata().unwrap().len() == target_path.metadata().unwrap().len() {
        println!("Skipping the file {:?} because they already existing file has the same size and is likely same.", source_path);
        return None;
    } else {
        let alternative_new_path = create_alternative_path(&target_path);
        println!("Choose a resolution:");
        println!(
            "1) Override the target file with the source file (Size {:?}).",
            human_bytes(
                source_path
                    .metadata()
                    .expect("File should always exist")
                    .len() as f64
            )
        );
        println!(
            "2) Skip the source file and keep the file (Size: {:?}) at the target location. ",
            human_bytes(
                target_path
                    .metadata()
                    .expect("File should always exist")
                    .len() as f64
            )
        );
        println!(
            "3) Both files. The source file would be renamed to {:?}",
            alternative_new_path
                .file_name()
                .expect("Should always be a valid filename")
                .to_str()
                .expect("Should always be a valid filename")
        );
        let answer = loop {
            let mut input = String::new();
            _ = std::io::stdin().read_line(&mut input);
            input = input.trim().to_string();
            if ["1", "2", "3"].contains(&input.as_str()) {
                println!("Your option: {}", input);
                break input;
            } else {
                println!("Invalid option {}. Choose 1, 2 or 3.", input)
            }
        };
        if "1" == answer {
            return Some(target_path.to_owned());
        } else if "2" == answer {
            println!("Skipping file {:?}", source_path);
            return None;
        } else if "3" == answer {
            return Some(alternative_new_path);
        } else {
            panic!("Unreachable.")
        }
    }
}

fn create_alternative_path(path: &PathBuf) -> PathBuf {
    let new_name = path
        .file_stem()
        .expect("Should always have a file stem.")
        .to_str()
        .expect("Should always have a file stem.")
        .to_owned()
        + "_new";
    let mut new_path = change_file_name(path, new_name.as_str());
    if new_path.exists() {
        new_path = create_alternative_path(&new_path);
    }
    new_path
}

fn change_file_name(path: &PathBuf, name: &str) -> PathBuf {
    let mut result = path.to_owned();
    result.set_file_name(name);
    if let Some(ext) = path.extension() {
        result.set_extension(ext);
    }
    result
}

enum Mode {
    DryRun,
    Move,
    Copy,
}
