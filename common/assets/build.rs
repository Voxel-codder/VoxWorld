use std::{
    collections::{BTreeMap, BTreeSet},
    env, fs, io,
    path::{Path, PathBuf},
};

#[derive(Clone, Eq, Ord, PartialEq, PartialOrd)]
enum Entry {
    Dir(String),
    File { id: String, ext: String },
}

fn main() -> io::Result<()> {
    let manifest_dir = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let repo_root = manifest_dir
        .parent()
        .and_then(Path::parent)
        .expect("common/assets should live two levels below the repository root");
    let assets_root = repo_root.join("assets");
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let generated_path = out_dir.join("embedded_assets.rs");

    println!(
        "cargo:rerun-if-changed={}",
        assets_root.join("world").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        assets_root.join("common").join("canary.canary").display()
    );

    let mut files = Vec::new();
    collect_embedded_world_files(&assets_root.join("world"), &mut files)?;
    files.push(assets_root.join("common").join("canary.canary"));
    files.sort();

    let mut dirs: BTreeMap<String, BTreeSet<Entry>> = BTreeMap::new();
    let mut file_rows = Vec::new();
    for file in files {
        let rel = file
            .strip_prefix(&assets_root)
            .expect("embedded asset should live under assets root");
        let ext = rel
            .extension()
            .and_then(|ext| ext.to_str())
            .expect("embedded asset should have an extension");
        let id_parts = rel
            .with_extension("")
            .components()
            .map(|component| component.as_os_str().to_string_lossy().replace('\\', "/"))
            .collect::<Vec<_>>();
        let id = id_parts.join(".");
        let include_path = file.to_string_lossy().replace('\\', "\\\\");

        file_rows.push(format!(
            "    (({id:?}, {ext:?}), include_bytes!({include_path:?}).as_slice()),"
        ));
        register_dir_entries(&mut dirs, &id_parts, ext);
    }

    let mut dir_defs = Vec::new();
    let mut dir_rows = Vec::new();
    for (dir_index, (dir, entries)) in dirs.iter().enumerate() {
        let const_name = format!("DIR_{dir_index}");
        let entries = entries
            .iter()
            .map(|entry| match entry {
                Entry::Dir(id) => format!("        DirEntry::Directory({id:?}),"),
                Entry::File { id, ext } => format!("        DirEntry::File({id:?}, {ext:?}),"),
            })
            .collect::<Vec<_>>()
            .join("\n");

        dir_defs.push(format!(
            "const {const_name}: &[DirEntry<'static>] = &[\n{entries}\n];"
        ));
        dir_rows.push(format!("    ({dir:?}, {const_name}),"));
    }

    let output = format!(
        "use assets_manager::source::{{DirEntry, RawEmbedded}};\n\n{}\n\nconst FILES: &[((&str, \
         &str), &[u8])] = &[\n{}\n];\n\nconst DIRS: &[(&str, &[DirEntry<'static>])] = \
         &[\n{}\n];\n\npub const RAW_EMBEDDED: RawEmbedded<'static> = RawEmbedded {{ files: \
         FILES, dirs: DIRS }};\n",
        dir_defs.join("\n\n"),
        file_rows.join("\n"),
        dir_rows.join("\n"),
    );
    fs::write(generated_path, output)
}

fn collect_embedded_world_files(dir: &Path, files: &mut Vec<PathBuf>) -> io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            collect_embedded_world_files(&path, files)?;
        } else if path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| matches!(ext, "ron" | "vox"))
        {
            files.push(path);
        }
    }
    Ok(())
}

fn register_dir_entries(
    dirs: &mut BTreeMap<String, BTreeSet<Entry>>,
    id_parts: &[String],
    ext: &str,
) {
    for parent_len in 0..id_parts.len() {
        let parent = id_parts[..parent_len].join(".");
        let child_id = id_parts[..=parent_len].join(".");
        let entry = if parent_len + 1 == id_parts.len() {
            Entry::File {
                id: child_id,
                ext: ext.to_owned(),
            }
        } else {
            Entry::Dir(child_id)
        };
        dirs.entry(parent).or_default().insert(entry);
    }
}
