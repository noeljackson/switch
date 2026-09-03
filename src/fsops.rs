use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};
use crate::paths::{clean, is_folder};

/// Copies a file or a whole directory tree, merging into whatever is already
/// at the destination.
///
/// Prefer [`replace_path`] for profile data: merging leaves files from the
/// previous profile in place.
pub fn copy_path(src: impl AsRef<Path>, dst: impl AsRef<Path>) -> Result<()> {
    let (src, dst) = (src.as_ref(), dst.as_ref());
    if is_folder(src) {
        copy_folder(src, dst)
    } else {
        copy_file(src, dst)
    }
}

/// Replaces `dst` with an exact copy of `src`.
///
/// Unlike [`copy_path`] this does not merge: anything at the destination that
/// the source does not have is gone afterwards. That is what profile switching
/// needs — restoring a profile that lacks a credentials file must not leave the
/// previous profile's credentials in place.
///
/// The copy is staged alongside the destination and swapped in with renames, so
/// an interrupted run leaves either the old content or the new, never a
/// half-written mixture.
pub fn replace_path(src: impl AsRef<Path>, dst: impl AsRef<Path>) -> Result<()> {
    let (src, dst) = (src.as_ref(), dst.as_ref());

    if overlaps(src, dst) {
        return Err(Error::new(format!(
            "refusing to copy {} into {}: one contains the other",
            src.display(),
            dst.display()
        )));
    }

    if let Some(parent) = dst.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|e| io_err("mkdir", parent, e))?;
        }
    }

    let staging = sibling(dst, "new");
    let _ = remove_path(&staging);
    if let Err(e) = copy_path(src, &staging) {
        let _ = remove_path(&staging);
        return Err(e);
    }

    let previous = sibling(dst, "old");
    let _ = remove_path(&previous);

    let had_destination = fs::symlink_metadata(dst).is_ok();
    if had_destination {
        if let Err(e) = fs::rename(dst, &previous) {
            let _ = remove_path(&staging);
            return Err(io_err("rename", dst, e));
        }
    }

    match fs::rename(&staging, dst) {
        Ok(()) => {
            if had_destination {
                let _ = remove_path(&previous);
            }
            Ok(())
        }
        Err(e) => {
            // Put the original back rather than leaving nothing behind.
            if had_destination {
                let _ = fs::rename(&previous, dst);
            }
            let _ = remove_path(&staging);
            Err(io_err("rename", dst, e))
        }
    }
}

/// Writes a file by staging a sibling and renaming over the target, so a crash
/// or a full disk cannot leave a truncated file. Used for `~/.switch.toml`,
/// where a partial write would lose every configured profile.
pub fn write_atomic(path: impl AsRef<Path>, contents: &[u8]) -> io::Result<()> {
    use std::io::Write;

    let path = path.as_ref();
    let staging = sibling(path, "new");

    {
        let mut file = fs::File::create(&staging)?;
        file.write_all(contents)?;
        file.sync_all()?;
    }

    match fs::rename(&staging, path) {
        Ok(()) => Ok(()),
        Err(e) => {
            let _ = fs::remove_file(&staging);
            Err(e)
        }
    }
}

/// Removes a file or directory tree. A missing path is not an error.
pub fn remove_path(path: impl AsRef<Path>) -> io::Result<()> {
    let path = path.as_ref();
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_dir() => fs::remove_dir_all(path),
        Ok(_) => fs::remove_file(path),
        Err(_) => Ok(()),
    }
}

/// Reports whether either path contains the other, or they are the same.
///
/// Copying between overlapping paths is never what the user meant, and for
/// directories it is actively destructive.
pub fn overlaps(a: impl AsRef<Path>, b: impl AsRef<Path>) -> bool {
    let a = clean(&a.as_ref().to_string_lossy());
    let b = clean(&b.as_ref().to_string_lossy());
    contains_or_equals(&a, &b) || contains_or_equals(&b, &a)
}

fn contains_or_equals(outer: &str, inner: &str) -> bool {
    if outer == inner {
        return true;
    }
    let prefix = if outer.ends_with('/') {
        outer.to_string()
    } else {
        format!("{outer}/")
    };
    inner.starts_with(&prefix)
}

/// A scratch path next to `path`, used for staging before a rename.
fn sibling(path: &Path, tag: &str) -> PathBuf {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned());
    let name = name.unwrap_or_else(|| "switch".to_string());
    let scratch = format!(".{name}.switch-{tag}-{}", std::process::id());
    match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.join(scratch),
        _ => PathBuf::from(scratch),
    }
}

/// Copies a single file, creating any missing parent directories and carrying
/// the source's permission bits across.
pub fn copy_file(src: impl AsRef<Path>, dst: impl AsRef<Path>) -> Result<()> {
    let (src, dst) = (src.as_ref(), dst.as_ref());

    let src_meta = fs::metadata(src).map_err(|e| io_err("stat", src, e))?;
    let perm = mode_of(&src_meta);

    let mut source = fs::File::open(src).map_err(|e| io_err("open", src, e))?;

    // `filepath.Dir` yields "." for a bare filename, which MkdirAll accepts;
    // Rust yields "", which `create_dir_all` rejects, so skip it.
    if let Some(parent) = dst.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|e| io_err("mkdir", parent, e))?;
        }
    }

    let mut destination = fs::File::create(dst).map_err(|e| io_err("open", dst, e))?;
    io::copy(&mut source, &mut destination).map_err(|e| io_err("copy", dst, e))?;

    // chmod explicitly, because the mode passed at creation is masked by umask.
    set_mode(dst, perm).map_err(|e| io_err("chmod", dst, e))?;
    Ok(())
}

/// Recursively copies a directory tree, merging into an existing destination.
pub fn copy_folder(src: impl AsRef<Path>, dst: impl AsRef<Path>) -> Result<()> {
    walk_copy(src.as_ref(), dst.as_ref())
}

fn walk_copy(src: &Path, dst: &Path) -> Result<()> {
    let meta = fs::symlink_metadata(src).map_err(|e| io_err("stat", src, e))?;

    if !meta.is_dir() {
        return copy_file(src, dst);
    }

    // The listing is snapshotted BEFORE the destination is created, which is
    // the order `filepath.Walk` uses. It matters when `dst` is nested inside
    // `src`: creating the destination first would put it in the listing and
    // the copy would descend into its own output. Walk visits entries in
    // lexical order.
    let mut entries: Vec<_> = fs::read_dir(src)
        .map_err(|e| io_err("readdir", src, e))?
        .collect::<io::Result<Vec<_>>>()
        .map_err(|e| io_err("readdir", src, e))?;
    entries.sort_by_key(|e| e.file_name());

    let mode = mode_of(&meta);
    fs::create_dir_all(dst).map_err(|e| io_err("mkdir", dst, e))?;
    set_mode(dst, mode).map_err(|e| io_err("chmod", dst, e))?;

    for entry in entries {
        let from = entry.path();
        // Never descend into the destination, however it was reached.
        if contains_or_equals(
            &clean(&from.to_string_lossy()),
            &clean(&dst.to_string_lossy()),
        ) {
            continue;
        }
        walk_copy(&from, &dst.join(entry.file_name()))?;
    }
    Ok(())
}

/// Formats an IO failure with the path that caused it.
fn io_err(op: &str, path: &Path, e: io::Error) -> Error {
    Error::new(format!("{op} {}: {e}", path.display()))
}

#[cfg(unix)]
fn mode_of(meta: &fs::Metadata) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    meta.permissions().mode() & 0o777
}

#[cfg(not(unix))]
fn mode_of(_meta: &fs::Metadata) -> u32 {
    0
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
}

/// On Windows `chmod` only toggles the read-only bit, which the copy already
/// reproduces, so there is nothing to do.
#[cfg(not(unix))]
fn set_mode(_path: &Path, _mode: u32) -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reports whether directory permissions actually deny this process. They
    /// do not when running as root.
    #[cfg(unix)]
    fn perms_enforced() -> bool {
        let dir = tempfile::tempdir().unwrap();
        let ro = dir.path().join("probe");
        fs::create_dir(&ro).unwrap();
        set_mode(&ro, 0o555).unwrap();
        fs::write(ro.join("x"), b"x").is_err()
    }

    fn tree(root: &Path) -> Vec<String> {
        let mut found = Vec::new();
        collect(root, root, &mut found);
        found.sort();
        found
    }

    fn collect(root: &Path, dir: &Path, out: &mut Vec<String>) {
        for entry in fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            let rel = path
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .into_owned();
            if path.is_dir() {
                out.push(format!("{rel}/"));
                collect(root, &path, out);
            } else {
                out.push(rel);
            }
        }
    }

    // Port of TestCopyFileFolderAndPath.
    #[test]
    fn copy_file_folder_and_path() {
        let base = tempfile::tempdir().unwrap();

        let src = base.path().join("a.txt");
        let dst = base.path().join("b.txt");
        fs::write(&src, b"hello").unwrap();
        copy_file(&src, &dst).unwrap();
        assert_eq!(fs::read(&dst).unwrap(), b"hello");

        let dsrc = base.path().join("dirsrc");
        let ddst = base.path().join("dirdst");
        fs::create_dir_all(dsrc.join("nested")).unwrap();
        fs::write(dsrc.join("nested").join("f.txt"), b"x").unwrap();
        copy_path(&dsrc, &ddst).unwrap();
        assert!(
            ddst.join("nested").join("f.txt").exists(),
            "copied file missing"
        );
    }

    // Port of TestCopyPreservesPermissions_FileAndDir (skipped on Windows there).
    #[cfg(unix)]
    #[test]
    fn copy_preserves_permissions_file_and_dir() {
        let base = tempfile::tempdir().unwrap();

        let src = base.path().join("fp.txt");
        fs::write(&src, b"data").unwrap();
        set_mode(&src, 0o640).unwrap();
        let dst = base.path().join("out").join("fp.txt");
        copy_file(&src, &dst).unwrap();
        assert_eq!(mode_of(&fs::metadata(&dst).unwrap()), 0o640);

        let src_dir = base.path().join("srcd");
        fs::create_dir_all(src_dir.join("n")).unwrap();
        fs::write(src_dir.join("n").join("f"), b"x").unwrap();
        set_mode(&src_dir.join("n"), 0o750).unwrap();
        let dst_dir = base.path().join("dstd");
        copy_folder(&src_dir, &dst_dir).unwrap();
        assert_eq!(mode_of(&fs::metadata(dst_dir.join("n")).unwrap()), 0o750);
    }

    // Port of TestCopyFile_Errors.
    #[test]
    fn copy_file_missing_source() {
        let base = tempfile::tempdir().unwrap();
        assert!(copy_file("/no/such/src", base.path().join("x")).is_err());
    }

    // Port of TestCopyFile_DestinationOpenError.
    #[cfg(unix)]
    #[test]
    fn copy_file_destination_open_error() {
        if !perms_enforced() {
            eprintln!("skipping: directory permissions are not enforced for this user");
            return;
        }
        let base = tempfile::tempdir().unwrap();
        let src = base.path().join("src.txt");
        fs::write(&src, b"x").unwrap();

        let ro = base.path().join("rodir");
        fs::create_dir_all(&ro).unwrap();
        set_mode(&ro, 0o555).unwrap();

        assert!(
            copy_file(&src, ro.join("dest.txt")).is_err(),
            "expected an error when the destination directory is not writable"
        );
    }

    // Port of TestCopyFile_MkdirAllError.
    #[test]
    fn copy_file_mkdir_all_error() {
        let base = tempfile::tempdir().unwrap();
        let src = base.path().join("src.txt");
        fs::write(&src, b"x").unwrap();

        let bad_dir = base.path().join("notadir");
        fs::write(&bad_dir, b"f").unwrap();

        let dst = bad_dir.join("child").join("dest.txt");
        assert!(
            copy_file(&src, &dst).is_err(),
            "expected MkdirAll to fail on a file path"
        );
    }

    // Port of TestCopyFolder_MkdirAllError.
    #[test]
    fn copy_folder_mkdir_all_error() {
        let base = tempfile::tempdir().unwrap();
        let src = base.path().join("srcd");
        fs::create_dir_all(src.join("sub")).unwrap();
        let dst = base.path().join("dstd");
        fs::create_dir_all(&dst).unwrap();
        fs::write(dst.join("sub"), b"x").unwrap();

        assert!(
            copy_folder(&src, &dst).is_err(),
            "expected MkdirAll to fail on an existing file"
        );
    }

    /// A destination nested inside the source must not be copied into itself.
    #[test]
    fn copy_folder_into_a_nested_destination_terminates() {
        let base = tempfile::tempdir().unwrap();
        let src = base.path().join("ssh");
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("id_rsa"), b"KEY").unwrap();
        fs::write(src.join("config"), b"cfg").unwrap();

        let dst = src.join("profiles").join("work.switch");
        copy_folder(&src, &dst).unwrap();

        assert_eq!(fs::read(dst.join("id_rsa")).unwrap(), b"KEY");
        assert_eq!(fs::read(dst.join("config")).unwrap(), b"cfg");
        assert!(
            !dst.join("profiles").exists(),
            "the copy descended into its own output"
        );
    }

    /// Even with the backup directory already present, a second copy must not
    /// pull the earlier backups into the new one.
    #[test]
    fn copy_folder_skips_the_destination_on_a_later_run() {
        let base = tempfile::tempdir().unwrap();
        let src = base.path().join("ssh");
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("id_rsa"), b"KEY").unwrap();

        copy_folder(&src, src.join("profiles").join("work.switch")).unwrap();
        copy_folder(&src, src.join("profiles").join("home.switch")).unwrap();

        let home = src.join("profiles").join("home.switch");
        assert_eq!(tree(&home), vec!["id_rsa".to_string()]);
    }

    #[test]
    fn copy_folder_merges_into_existing_destination() {
        let base = tempfile::tempdir().unwrap();
        let src = base.path().join("s");
        let dst = base.path().join("d");
        fs::create_dir_all(&src).unwrap();
        fs::create_dir_all(&dst).unwrap();
        fs::write(src.join("new.txt"), b"new").unwrap();
        fs::write(dst.join("keep.txt"), b"keep").unwrap();

        copy_folder(&src, &dst).unwrap();

        assert_eq!(fs::read(dst.join("new.txt")).unwrap(), b"new");
        assert!(
            dst.join("keep.txt").exists(),
            "merge must keep existing files"
        );
    }

    #[test]
    fn replace_path_drops_files_the_source_does_not_have() {
        let base = tempfile::tempdir().unwrap();
        let src = base.path().join("personal");
        let dst = base.path().join("live");
        fs::create_dir_all(&src).unwrap();
        fs::create_dir_all(&dst).unwrap();
        fs::write(src.join("id.txt"), b"personal").unwrap();
        fs::write(dst.join("id.txt"), b"work").unwrap();
        fs::write(dst.join("credentials"), b"work-token").unwrap();

        replace_path(&src, &dst).unwrap();

        assert_eq!(fs::read(dst.join("id.txt")).unwrap(), b"personal");
        assert!(
            !dst.join("credentials").exists(),
            "a file absent from the source must not survive the replace"
        );
    }

    #[test]
    fn replace_path_handles_files_and_a_missing_destination() {
        let base = tempfile::tempdir().unwrap();
        let src = base.path().join("a.json");
        fs::write(&src, b"{}").unwrap();

        let fresh = base.path().join("sub").join("b.json");
        replace_path(&src, &fresh).unwrap();
        assert_eq!(fs::read(&fresh).unwrap(), b"{}");

        fs::write(&src, b"{\"v\":2}").unwrap();
        replace_path(&src, &fresh).unwrap();
        assert_eq!(fs::read(&fresh).unwrap(), b"{\"v\":2}");
    }

    #[test]
    fn replace_path_leaves_no_staging_files_behind() {
        let base = tempfile::tempdir().unwrap();
        let src = base.path().join("src");
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("f"), b"x").unwrap();
        let dst = base.path().join("dst");

        replace_path(&src, &dst).unwrap();
        replace_path(&src, &dst).unwrap();

        let leftovers: Vec<_> = tree(base.path())
            .into_iter()
            .filter(|p| p.contains("switch-new") || p.contains("switch-old"))
            .collect();
        assert!(
            leftovers.is_empty(),
            "staging paths left behind: {leftovers:?}"
        );
    }

    #[test]
    fn replace_path_refuses_overlapping_paths() {
        let base = tempfile::tempdir().unwrap();
        let src = base.path().join("ssh");
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("f"), b"x").unwrap();

        let err = replace_path(&src, src.join("profiles").join("w.switch")).unwrap_err();
        assert!(err.to_string().contains("one contains the other"), "{err}");
    }

    #[test]
    fn overlaps_detects_containment_either_way() {
        assert!(overlaps("/a/b", "/a/b"));
        assert!(overlaps("/a/b", "/a/b/c"));
        assert!(overlaps("/a/b/c", "/a/b"));
        assert!(!overlaps("/a/b", "/a/bc"));
        assert!(!overlaps("/a/b", "/a/c"));
        assert!(!overlaps("/a/b", "/x"));
    }

    #[test]
    fn write_atomic_replaces_the_target() {
        let base = tempfile::tempdir().unwrap();
        let path = base.path().join("cfg.toml");

        write_atomic(&path, b"first").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"first");

        write_atomic(&path, b"second").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"second");

        assert_eq!(tree(base.path()), vec!["cfg.toml".to_string()]);
    }

    #[test]
    fn write_atomic_fails_without_destroying_the_original() {
        let base = tempfile::tempdir().unwrap();
        let dir = base.path().join("adir");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("keep"), b"keep").unwrap();

        // Renaming a file over a non-empty directory fails.
        assert!(write_atomic(&dir, b"x").is_err());
        assert_eq!(fs::read(dir.join("keep")).unwrap(), b"keep");
    }
}
