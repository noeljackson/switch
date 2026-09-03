use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use crate::paths::is_folder;

/// Compares a live config against a stored profile.
///
/// Port of `contentEqual`: two directories are compared as folders, two
/// non-directories as files, and a mismatch in kind is never equal.
pub fn content_equal(a: impl AsRef<Path>, b: impl AsRef<Path>) -> bool {
    let (a, b) = (a.as_ref(), b.as_ref());
    match (is_folder(a), is_folder(b)) {
        (true, true) => folder_equal(a, b),
        (false, false) => file_equal(a, b),
        _ => false,
    }
}

/// Compares two files, treating them as JSON when both parse as JSON objects
/// so that key order does not matter, and byte-for-byte otherwise.
///
/// Port of `fileEqual`. Go decoded into `map[string]interface{}`, which only
/// accepts an object or `null`; anything else fails to decode and falls back to
/// a raw comparison. That distinction is reproduced by [`parse_json_map`].
pub fn file_equal(a: impl AsRef<Path>, b: impl AsRef<Path>) -> bool {
    let Ok(a_data) = std::fs::read(a.as_ref()) else {
        return false;
    };
    let Ok(b_data) = std::fs::read(b.as_ref()) else {
        return false;
    };

    if let (Some(a_json), Some(b_json)) = (parse_json_map(&a_data), parse_json_map(&b_data)) {
        return json_equal(&a_json, &b_json);
    }

    a_data == b_data
}

/// Compares two directory trees: same set of relative paths, and every file
/// equal by [`file_equal`].
///
/// The Go original only checked that both paths were directories, which made
/// every folder-backed profile look identical. That left `switch list` naming
/// the wrong profile as current, and cycling stuck between the first two.
///
/// Contents are compared file by file rather than by size, because
/// [`file_equal`] treats JSON as order-insensitive and two equal configs can
/// differ in length.
pub fn folder_equal(a: impl AsRef<Path>, b: impl AsRef<Path>) -> bool {
    let (a, b) = (a.as_ref(), b.as_ref());

    if !is_folder(a) || !is_folder(b) {
        return false;
    }

    let (Some(a_entries), Some(b_entries)) = (relative_tree(a), relative_tree(b)) else {
        return false;
    };
    if a_entries != b_entries {
        return false;
    }

    a_entries
        .iter()
        .filter(|(_, kind)| *kind == EntryKind::File)
        .all(|(rel, _)| file_equal(a.join(rel), b.join(rel)))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum EntryKind {
    File,
    Dir,
}

/// Every path under `root`, relative to it, sorted so two trees compare
/// directly. `None` if the tree cannot be read.
fn relative_tree(root: &Path) -> Option<Vec<(PathBuf, EntryKind)>> {
    let mut found = Vec::new();
    collect_tree(root, root, &mut found).ok()?;
    found.sort();
    Some(found)
}

fn collect_tree(
    root: &Path,
    dir: &Path,
    out: &mut Vec<(PathBuf, EntryKind)>,
) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let Ok(rel) = path.strip_prefix(root) else {
            continue;
        };
        // Symlinks are compared as files, by what they point at.
        if entry.file_type()?.is_dir() {
            out.push((rel.to_path_buf(), EntryKind::Dir));
            collect_tree(root, &path, out)?;
        } else {
            out.push((rel.to_path_buf(), EntryKind::File));
        }
    }
    Ok(())
}

/// Decodes JSON that Go would have accepted into a `map[string]interface{}`.
///
/// `Some(None)` is a literal `null` (Go leaves the map nil), `Some(Some(_))` is
/// an object, and `None` means the input is something else entirely — an array,
/// a scalar, or malformed — in which case the caller compares raw bytes.
fn parse_json_map(data: &[u8]) -> Option<Option<Map<String, Value>>> {
    serde_json::from_slice::<Option<Map<String, Value>>>(data).ok()
}

/// Compares two decoded JSON objects the way Go's marshal-and-compare did.
///
/// `serde_json::Map` is ordered, so a structural comparison already matches
/// Go's sorted-key encoding. Numbers need normalising first: Go decodes every
/// JSON number as `float64`, making `1` and `1.0` equal, while `serde_json`
/// keeps them as distinct variants.
fn json_equal(a: &Option<Map<String, Value>>, b: &Option<Map<String, Value>>) -> bool {
    match (a, b) {
        // Two nil maps both encode as `null`.
        (None, None) => true,
        // A nil map encodes as `null`, never as `{}`.
        (None, Some(_)) | (Some(_), None) => false,
        (Some(a), Some(b)) => {
            let mut a = Value::Object(a.clone());
            let mut b = Value::Object(b.clone());
            normalize_numbers(&mut a);
            normalize_numbers(&mut b);
            a == b
        }
    }
}

fn normalize_numbers(value: &mut Value) {
    match value {
        Value::Number(n) => {
            if let Some(f) = n.as_f64() {
                if let Some(normalized) = serde_json::Number::from_f64(f) {
                    *value = Value::Number(normalized);
                }
            }
        }
        Value::Array(items) => items.iter_mut().for_each(normalize_numbers),
        Value::Object(map) => map.values_mut().for_each(normalize_numbers),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Port of TestEqualFunctions.
    #[test]
    fn equal_functions() {
        let dir = tempfile::tempdir().unwrap();

        // JSON comparison ignores key order.
        let f1 = dir.path().join("a.json");
        let f2 = dir.path().join("b.json");
        std::fs::write(&f1, br#"{"k":1, "z":2}"#).unwrap();
        std::fs::write(&f2, br#"{"z":2, "k":1}"#).unwrap();
        assert!(file_equal(&f1, &f2), "fileEqual json should be true");

        // Plain text falls back to a byte comparison.
        let t1 = dir.path().join("a.txt");
        let t2 = dir.path().join("b.txt");
        std::fs::write(&t1, b"abc").unwrap();
        std::fs::write(&t2, b"abc").unwrap();
        assert!(file_equal(&t1, &t2), "fileEqual text should be true");

        let d1 = dir.path().join("d1");
        let d2 = dir.path().join("d2");
        std::fs::create_dir_all(&d1).unwrap();
        std::fs::create_dir_all(&d2).unwrap();
        assert!(
            folder_equal(&d1, &d2),
            "folderEqual should be true for dirs"
        );

        assert!(content_equal(&t1, &t2), "contentEqual files should be true");
        assert!(content_equal(&d1, &d2), "contentEqual dirs should be true");
    }

    // Port of TestFileEqual_NonJSON_NotEqual.
    #[test]
    fn file_equal_non_json_not_equal() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.txt");
        let b = dir.path().join("b.txt");
        std::fs::write(&a, b"aaa").unwrap();
        std::fs::write(&b, b"bbb").unwrap();
        assert!(
            !file_equal(&a, &b),
            "expected not equal for different text files"
        );
    }

    // Port of TestContentAndFileFolderEqual_Negatives.
    #[test]
    fn content_and_file_folder_equal_negatives() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("f");
        let d = dir.path().join("d");
        std::fs::write(&f, b"x").unwrap();
        std::fs::create_dir_all(&d).unwrap();

        assert!(
            !content_equal(&f, &d),
            "contentEqual should be false for file vs dir"
        );
        assert!(
            !file_equal("/nope/a", "/nope/b"),
            "fileEqual missing files should be false"
        );
        assert!(
            !folder_equal("/nope/a", &d),
            "folderEqual missing should be false"
        );
    }

    #[test]
    fn folder_equal_compares_contents() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a");
        let b = dir.path().join("b");
        for root in [&a, &b] {
            std::fs::create_dir_all(root.join("nested")).unwrap();
            std::fs::write(root.join("nested").join("f.txt"), b"same").unwrap();
        }
        assert!(folder_equal(&a, &b), "identical trees should be equal");

        std::fs::write(b.join("nested").join("f.txt"), b"different").unwrap();
        assert!(
            !folder_equal(&a, &b),
            "differing file contents must not be equal"
        );
    }

    #[test]
    fn folder_equal_notices_extra_and_missing_entries() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a");
        let b = dir.path().join("b");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        std::fs::write(a.join("shared"), b"x").unwrap();
        std::fs::write(b.join("shared"), b"x").unwrap();
        assert!(folder_equal(&a, &b));

        // A credentials file present in only one tree is exactly the case that
        // used to go unnoticed.
        std::fs::write(a.join("credentials"), b"token").unwrap();
        assert!(!folder_equal(&a, &b));

        std::fs::write(b.join("credentials"), b"token").unwrap();
        assert!(folder_equal(&a, &b));

        // A directory on one side, a file of the same name on the other.
        std::fs::create_dir_all(a.join("thing")).unwrap();
        std::fs::write(b.join("thing"), b"").unwrap();
        assert!(!folder_equal(&a, &b));
    }

    #[test]
    fn folder_equal_applies_json_rules_to_nested_files() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a");
        let b = dir.path().join("b");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        std::fs::write(a.join("settings.json"), br#"{"x":1,"y":2}"#).unwrap();
        std::fs::write(b.join("settings.json"), br#"{"y":2,"x":1}"#).unwrap();

        assert!(
            folder_equal(&a, &b),
            "key order should not matter inside a folder"
        );
    }

    #[test]
    fn json_comparison_matches_go_decoding_rules() {
        let dir = tempfile::tempdir().unwrap();
        let write = |name: &str, body: &str| {
            let p = dir.path().join(name);
            std::fs::write(&p, body.as_bytes()).unwrap();
            p
        };

        // Go decodes every number as float64, so these are equal.
        assert!(file_equal(
            write("i.json", r#"{"a":1}"#),
            write("f.json", r#"{"a":1.0}"#)
        ));

        // Nested objects compare regardless of key order.
        assert!(file_equal(
            write("n1.json", r#"{"o":{"x":1,"y":2}}"#),
            write("n2.json", r#"{"o":{"y":2,"x":1}}"#),
        ));

        // `null` decodes to a nil map, which does not equal an empty object.
        assert!(file_equal(
            write("z1.json", "null"),
            write("z2.json", " null ")
        ));
        assert!(!file_equal(
            write("z3.json", "null"),
            write("e1.json", "{}")
        ));

        // Arrays never decode into a map, so they compare byte-for-byte and
        // element order matters.
        assert!(file_equal(
            write("a1.json", "[1,2]"),
            write("a2.json", "[1,2]")
        ));
        assert!(!file_equal(
            write("a3.json", "[1,2]"),
            write("a4.json", "[2,1]")
        ));

        // Two files that both fail to parse still match if their bytes match.
        assert!(file_equal(
            write("b1.txt", "not json"),
            write("b2.txt", "not json")
        ));

        // One valid object against one invalid file falls back to bytes.
        assert!(!file_equal(
            write("v.json", r#"{"a":1}"#),
            write("i.txt", "nope")
        ));
    }
}
