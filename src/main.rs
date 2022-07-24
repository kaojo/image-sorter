use chrono::{Datelike, NaiveDateTime};
use exif::{In, Tag};
use human_bytes::human_bytes;
use std::collections::HashSet;
use std::env;
use std::ffi::OsStr;
use std::fs::{self, DirEntry};
use std::io;
use std::path::{Path, PathBuf};
use std::process::exit;

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
            mode = Mode::Copy;
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

    let mut source_image_paths: HashSet<PathBuf> = HashSet::new();
    let mut errors = vec![];

    visit_dirs(
        &source_directory,
        &mut find_and_filter_images(verbose, &mut source_image_paths),
    )
    .unwrap();

    let exifreader = exif::Reader::new();

    for source_path in source_image_paths {
        println!("---------------");
        let date_time_res = extract_date_time(&source_path, &exifreader);

        match date_time_res {
            Ok(date_time) => {
                if verbose {
                    println!(
                        "Image {:?} was taken at DateTime {}",
                        source_path, date_time
                    )
                }
                let mut new_path = target_directory
                    .join(date_time.year().to_string())
                    .join(date_time.month().to_string())
                    .join(
                        source_path
                            .file_name()
                            .expect("we only supply valid files."),
                    );

                if new_path.exists() {
                    println!("Filename collision detected.");
                    println!(
                        "The file {:?} already exists at target {:?}",
                        source_path, new_path
                    );
                    if source_path.metadata().unwrap().len() == new_path.metadata().unwrap().len() {
                        println!("Skipping the file {:?} because they already exisintg file has the same size and is likely same.", source_path)
                    } else {
                        let alternative_new_path = create_alternative_path(&new_path);
                        println!(
                            "Choose a resolution:"
                        );
                        println!(
                            "1) Override the target file with the source file (Size {:?}).",
                            human_bytes(
                                source_path.metadata().expect("File should always exist").len() as f64
                            )
                        );
                        println!(
                            "2) Skip the source file and keep the file (Size: {:?}) at the target location. ",
                            human_bytes(
                                new_path.metadata().expect("File should always exist").len() as f64
                            )
                        );
                        println!(
                            "3) Both files. The source file would be renamed to {:?}",
                            alternative_new_path.file_name().expect("Should always be a valid filename").to_str().expect("Should always be a valid filename")
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
                        if "2" == answer {
                            println!("Skipping file {:?}", source_path);
                            continue;
                        } else if "3" == answer {
                            new_path = alternative_new_path;
                        }
                    }
                }
                match mode {
                    Mode::DryRun => println!(
                        "Dry run: Copy/Move source file {:?} to target {:?}",
                        source_path, new_path
                    ),
                    Mode::Move => todo!(),
                    Mode::Copy => todo!(),
                }
                println!("---------------");
            }
            Err(e) => {
                errors.push(
                    "Error in ".to_owned() + source_path.to_str().unwrap() + ": " + e.as_str(),
                );
            }
        }
    }

    errors.iter().for_each(|e| println!("{}", e))
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

fn exit_with_message<T>(message: &str) -> T {
    println!("{}", message);
    exit(1);
}

fn extract_date_time(path: &PathBuf, exifreader: &exif::Reader) -> Result<NaiveDateTime, String> {
    let date_time = std::fs::File::open(path)
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
        });
    date_time
}

fn find_and_filter_images<'a>(
    verbose: bool,
    source_image_paths: &'a mut HashSet<PathBuf>,
) -> impl FnMut(&DirEntry) + 'a {
    move |dir_entry: &DirEntry| -> () {
        let path = dir_entry.path();
        let option = path
            .extension()
            .and_then(OsStr::to_str)
            .filter(|&e| HashSet::from(["png", "jpg", "tif"]).contains(e));

        match option {
            Some(extension) => {
                if verbose {
                    println!(
                        "found extension {} for file {}",
                        extension,
                        dir_entry.path().to_str().unwrap()
                    );
                }
                source_image_paths.insert(path.canonicalize().unwrap());
            }
            None => (),
        }
    }
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
