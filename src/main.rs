use exif::{In, Tag};
use std::collections::HashSet;
use std::env;
use std::ffi::OsStr;
use std::fs::{self, DirEntry};
use std::io;
use std::path::{Path, PathBuf};

fn main() {
    let mut args: Vec<String> = env::args().collect();
    args.remove(0);

    let mut verbose: bool = false;
    let mut dryRun: bool = true;
    let mut folder = Option::None;
    for arg in args {
        if arg == "--verbose" || arg == "-v" {
            verbose = true;
        } else if arg == "--dry-run" || arg == "-d" {
            dryRun = true;
        } else {
            if folder.is_none() {
                folder = Option::Some(arg);
            } else {
                panic!("to many arguments given.")
            }
        }
    }

    let directory = Path::new(folder.get_or_insert(".".to_string()));

    let mut source_image_paths: HashSet<PathBuf> = HashSet::new();

    visit_dirs(
        &directory,
        &mut test_some_stuff_funtion(verbose, &mut source_image_paths),
    )
    .unwrap();

    if verbose {
        println!("Found the following images {:?}", source_image_paths);
    }

    for path in source_image_paths {
        let exifreader = exif::Reader::new();
        let exif_data = std::fs::File::open(&path)
            .map_err(|e| e.to_string())
            .map(|inner| std::io::BufReader::new(inner))
            .and_then(|mut inner| {
                exifreader
                    .read_from_container(&mut inner)
                    .map_err(|e| e.to_string())
            });

        match exif_data {
            Ok(exif) => {
                match exif
                    .get_field(Tag::DateTime, In::PRIMARY)
                    .or(exif.get_field(Tag::DateTimeOriginal, In::PRIMARY))
                    .or(exif.get_field(Tag::DateTimeDigitized, In::PRIMARY))
                {
                    Some(date) => {
                        println!("Image {:?} was taken {}", path, date.display_value().with_unit(&exif))
                    }
                    None => eprintln!("Error in {:?}: DateTime tag is missing", &path),
                }
            }
            Err(e) => {
                eprintln!("Error in {:?}: {:?}", &path, e)
            }
        }
    }
}

fn test_some_stuff_funtion(
    verbose: bool,
    source_image_paths: &mut HashSet<PathBuf>,
) -> impl FnMut(&DirEntry) + '_ {
    move |dir_entry: &DirEntry| -> () {
        let path = dir_entry.path();
        let option = path
            .extension()
            .and_then(OsStr::to_str)
            .filter(|&e| HashSet::from(["png", "jpg", "tif"]).contains(e));

        if let Some(extension) = option {
            if verbose {
                println!(
                    "found extension {} for file {}",
                    extension,
                    dir_entry.path().to_str().unwrap()
                );
            }
            source_image_paths.insert(path.canonicalize().unwrap());
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
