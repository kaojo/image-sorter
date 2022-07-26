use chrono::{DateTime, Datelike, NaiveDate, NaiveDateTime, NaiveTime};
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
    let args: Vec<String> = env::args().collect();
    let options: Options = parse_options(args);

    let date_regex = Regex::new(r"(?P<y>20[012]\d)\-?(?P<m>[01]\d)\-?(?P<d>\d{2})").unwrap();
    let mut target_parents = HashSet::new();

    visit_dirs(
        &options.source_folder,
        &mut handle_file(&options, &mut target_parents, &date_regex),
    )
    .unwrap();
}

struct Options {
    pub verbose: bool,
    pub mode: Mode,
    pub source_folder: PathBuf,
    pub target_folder: PathBuf,
    pub include_unsupported_file_types: bool,
    pub file_conflict_resolution_mode: FileConflictResolutionMode,
    pub media_creation_date_file_creation_fallback: bool,
    pub delete_skipped_source_duplicates: bool,
}

fn parse_options(args: Vec<String>) -> Options {
    let mut mode = Mode::Move;
    let mut mode_set = false;
    let mut verbose: bool = false;
    let mut source_folder_str = Option::None;
    let mut target_folder_str = Option::None;
    let mut file_conflict_resolution_mode = FileConflictResolutionMode::Choose;
    let mut media_creation_date_file_creation_fallback = false;
    let mut delete_skipped_source_duplicates = false;
    let mut include_unsupported_file_types = false;

    let mut skip_read_next_value = true;
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
            target_folder_str = args.get(i + 1);
            skip_read_next_value = true;
        } else if arg == "--conflict-mode" || arg == "-k" {
            let cm = args.get(i + 1).map(|s| s.as_str());
            file_conflict_resolution_mode = match cm {
                Some("both") => FileConflictResolutionMode::KeepBoth,
                Some("source") => FileConflictResolutionMode::KeepSource,
                Some("target") => FileConflictResolutionMode::KeepTarget,
                _ => FileConflictResolutionMode::Choose,
            };
            skip_read_next_value = true;
        } else if arg == "--file-creation-fallback" || arg == "-s" {
            media_creation_date_file_creation_fallback = true
        } else if arg == "--delete-skipped-source-duplicates" || arg == "-q" {
            delete_skipped_source_duplicates = true
        } else if arg == "--include-unsupported-file-types" || arg == "-u" {
            include_unsupported_file_types = true
        } else {
            if source_folder_str.is_none() {
                source_folder_str = Option::Some(arg.to_owned());
            } else {
                exit_with_message::<bool>("Too many arguments given.");
            }
        }
    }

    let source_folder = Path::new(source_folder_str.get_or_insert(".".to_string())).to_path_buf();
    let target_folder = target_folder_str
        .map(|s| Path::new(s))
        .unwrap_or_else(|| exit_with_message("No target folder supplied."))
        .to_path_buf();

    if !target_folder.exists() {
        exit_with_message::<bool>(
            "Target folder does not exists or you are missing the required permissions.",
        );
    }

    Options {
        verbose,
        mode,
        source_folder,
        target_folder,
        file_conflict_resolution_mode,
        media_creation_date_file_creation_fallback,
        delete_skipped_source_duplicates,
        include_unsupported_file_types,
    }
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
    options: &'a Options,
    target_parents: &'a mut HashSet<PathBuf>,
    date_regex: &'a Regex,
) -> impl FnMut(&DirEntry) + 'a {
    move |dir_entry: &DirEntry| -> () {
        let source_path = dir_entry.path();

        if is_supported_file_type(&source_path) || options.include_unsupported_file_types {
            let date_time = extract_date_time(
                &source_path,
                date_regex,
                options.media_creation_date_file_creation_fallback,
                options.verbose,
            );
            match date_time.and_then(|date_time| {
                sort_file(options, &source_path, target_parents, &date_time)
            }) {
                Ok(Some(target_file)) => {
                    let parent = target_file
                        .parent()
                        .expect("File and parent exist")
                        .to_owned()
                        .clone();
                    if !target_parents.contains(&parent) {
                        target_parents.insert(parent);
                    }
                }
                Ok(None) => {
                    if options.verbose {
                        println!("Skipped file.");
                    }
                }
                Err(e) => {
                    println!("Error in {:?}: {}", source_path, e);
                }
            }
        } else {
            if options.verbose {
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
        .filter(|&e| ["png", "jpg", "jpeg", "tif", "mp4", "mov"].contains(&e.to_lowercase().as_str()))
        .is_some();
    is_supported
}

fn sort_file(
    options: &Options,
    source_path: &PathBuf,
    target_parents: &HashSet<PathBuf>,
    date_time: &NaiveDateTime,
) -> Result<Option<PathBuf>, String> {
    println!("---------------");
    if options.verbose {
        println!("Found file {:?}.", source_path);
    }
    let target_path_unverified = options
        .target_folder
        .join(date_time.year().to_string())
        .join(date_time.month().to_string())
        .join(
            source_path
                .file_name()
                .expect("we only supply valid files."),
        );

    let path_check_result =
        validate_and_resolve_path_problems(options, target_path_unverified, source_path)?;
    match path_check_result {
        Some(valid_path) => {
            match options.mode {
                Mode::DryRun => println!(
                    "Dry run: Copy/Move source file {:?} to target {:?}",
                    source_path, valid_path
                ),
                Mode::Move => {
                    handle_missing_parents(options.verbose, &valid_path, target_parents)?;
                    if options.verbose {
                        println!(
                            "Moving source file {:?} to target {:?}",
                            source_path, valid_path
                        );
                    }
                    fs::rename(&source_path, &valid_path).map_err(|e| e.to_string())?;
                }
                Mode::Copy => {
                    handle_missing_parents(options.verbose, &valid_path, target_parents)?;
                    if options.verbose {
                        println!(
                            "Copying source file {:?} to target {:?}",
                            source_path, valid_path
                        );
                    }
                    fs::copy(&source_path, &valid_path).map_err(|e| e.to_string())?;
                }
            }
            Ok(Some(valid_path))
        }
        None => Ok(None),
    }
}

fn validate_and_resolve_path_problems(
    options: &Options,
    target_path_unverified: PathBuf,
    source_path: &PathBuf,
) -> Result<Option<PathBuf>, String> {
    if target_path_unverified.exists() {
        match handle_file_exists_at_target(
            &source_path,
            &target_path_unverified,
            &options.file_conflict_resolution_mode,
            options.verbose,
        ) {
            Some(path_resolution) => {
                validate_and_resolve_path_problems(options, path_resolution, source_path)
            }
            None => {
                // None means file move/copy is skipped
                if options.delete_skipped_source_duplicates {
                    match options.mode {
                        Mode::DryRun => {
                            println!("Dry run: Deleting skipped source file {:?}", source_path)
                        }
                        Mode::Move => {
                            if options.verbose {
                                println!("Deleting skipped source file {:?}", source_path);
                            }
                            fs::remove_file(source_path).map_err(|e| e.to_string())?;
                        }
                        _ => {}
                    }
                }
                Ok(None)
            }
        }
    } else {
        Ok(Some(target_path_unverified))
    }
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

fn extract_date_time(
    path: &PathBuf,
    date_regex: &Regex,
    file_creation_fallback: bool,
    verbose: bool,
) -> Result<NaiveDateTime, String> {
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
    } else if is_video(path) {
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
    } else {
        Err("Unsupported File Type".to_string())
    };

    let result = result_from_media_metadata
        .ok()
        .or_else(extract_media_creation_time_from_filename(date_regex, path))
        .or_else(extract_media_creation_time_from_file_metadata(
            path,
            file_creation_fallback,
        ))
        .ok_or("Could not determine a media file creation date.".to_owned());

    if let Ok(date_time) = result {
        if verbose {
            println!("Image {:?} was taken at DateTime {}", path, date_time)
        }
    }
    result
}

fn extract_media_creation_time_from_filename<'a>(
    date_regex: &'a Regex,
    path: &'a PathBuf,
) -> impl FnOnce() -> Option<NaiveDateTime> + 'a {
    || {
        let file_name = &path.file_name().map(|s| s.to_str()).unwrap().unwrap();
        match date_regex.captures_iter(file_name).count() {
            1 => date_regex
                .captures(file_name)
                .filter(|c| c.len() == 4)
                .and_then(|c| {
                    c.name("y")
                        .and_then(|s| s.as_str().parse::<i32>().ok())
                        .zip(c.name("m").and_then(|s| s.as_str().parse::<i32>().ok()))
                })
                .map(|s| {
                    NaiveDateTime::new(
                        NaiveDate::from_ymd(s.0, s.1 as u32, 1),
                        NaiveTime::from_hms(0, 0, 0),
                    )
                }),
            _ => None,
        }
    }
}

fn extract_media_creation_time_from_file_metadata<'a>(
    path: &'a PathBuf,
    file_creation_fallback: bool,
) -> impl FnOnce() -> Option<NaiveDateTime> + 'a {
    move || {
        let file_creation_date = path
            .metadata()
            .and_then(|m| m.modified().or(m.created()))
            .ok()
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .and_then(|duration| NaiveDateTime::from_timestamp_opt(duration.as_secs() as i64, 0));

        if file_creation_fallback && file_creation_date.is_some() {
            return file_creation_date;
        }

        match file_creation_date {
            Some(date) => {
                println!(
                    "Could not determine creation time of media file {:?}",
                    &path
                );
                println!("Choose a resolution:");
                println!("1) Use the file creation time: {:?}", date);
                println!("2) Enter year and month manually.");
                println!("3) Skip file. (it will not be deleted if the delete-skipped-source-duplicates flag is set.)");
                let answer = loop {
                    let mut input = String::new();
                    _ = std::io::stdin().read_line(&mut input);
                    input = input.trim().to_string();
                    if ["1", "2", "3"].contains(&input.as_str()) {
                        println!("Your option: {}", input);
                        break input;
                    } else {
                        println!("Invalid option {}. Choose 1, 2 or 3", input)
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
                            println!(
                                "Invalid input {}. expected a 4 digit number, e.g. 2022",
                                input
                            );
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
                            println!(
                                "Invalid input {}. expected a two digit number, e.g. 12",
                                input
                            );
                            continue;
                        }
                        if let Ok(month) = input.parse::<u32>() {
                            println!("Your option: {}", input);
                            break month;
                        } else {
                            println!("Invalid input {}. expected a number, e.g. 12", input)
                        }
                    };
                    return Some(NaiveDateTime::new(
                        NaiveDate::from_ymd(year, month, 1),
                        NaiveTime::from_hms(0, 0, 0),
                    ));
                } else if "3" == answer {
                    None
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

fn is_video(path: &PathBuf) -> bool {
    path.extension()
        .and_then(OsStr::to_str)
        .filter(|&e| ["mp4", "mov"].contains(&e.to_lowercase().as_str()))
        .is_some()
}

fn handle_file_exists_at_target(
    source_path: &PathBuf,
    target_path: &PathBuf,
    conflict_mode: &FileConflictResolutionMode,
    verbose: bool,
) -> Option<PathBuf> {
    println!("Filename collision detected.");
    println!(
        "The file {:?} already exists at target {:?}",
        source_path, target_path
    );
    if source_path.metadata().unwrap().len() == target_path.metadata().unwrap().len() {
        if verbose {
            println!("Skipping the file {:?} because they already existing file has the same size and is likely same.", source_path);
        }
        return None;
    } else {
        let alternative_new_path = create_alternative_path(&target_path);
        match conflict_mode {
            FileConflictResolutionMode::Choose => {
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
                "2) Skip the source file and keep the file (Size: {:?}) at the target location. (will delete source file if delete-skipped-source-duplicates flag is set)",
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
                    if verbose {
                        println!("Skipping file {:?}", source_path);
                    }
                    return None;
                } else if "3" == answer {
                    return Some(alternative_new_path);
                } else {
                    panic!("Unreachable.")
                }
            }
            FileConflictResolutionMode::KeepSource => Some(target_path.to_owned()),
            FileConflictResolutionMode::KeepTarget => None,
            FileConflictResolutionMode::KeepBoth => Some(alternative_new_path),
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
    change_file_name(path, new_name.as_str())
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

enum FileConflictResolutionMode {
    Choose,
    KeepSource,
    KeepTarget,
    KeepBoth,
}
