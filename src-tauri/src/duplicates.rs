use crate::{models::DuplicateGroup, scanner::hash_file};
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, fs, path::Path};

const MAX_DELETE_FILES: usize = 200;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DuplicateDeleteGroup {
    pub content_hash: String,
    pub paths: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DuplicateDeleteRequest {
    pub groups: Vec<DuplicateDeleteGroup>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeletedDuplicateFile {
    pub path: String,
    pub content_hash: String,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DuplicateDeleteFailure {
    pub path: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DuplicateDeleteResult {
    pub deleted_files: Vec<DeletedDuplicateFile>,
    pub removed_bytes: u64,
    pub failures: Vec<DuplicateDeleteFailure>,
}

#[derive(Debug, Clone)]
struct DeletePlan {
    group: DuplicateGroup,
    paths: Vec<String>,
}

pub fn delete_files(
    groups: &mut Vec<DuplicateGroup>,
    request: DuplicateDeleteRequest,
) -> Result<DuplicateDeleteResult, String> {
    let plans = validate_request(groups, request)?;
    let mut deleted_files = Vec::new();
    let mut failures = Vec::new();

    for plan in plans {
        let requested: HashSet<&str> = plan.paths.iter().map(String::as_str).collect();
        let retained_is_valid = plan
            .group
            .files
            .iter()
            .filter(|file| !requested.contains(file.path.as_str()))
            .any(|file| {
                verify_duplicate(&file.path, plan.group.size_bytes, &plan.group.content_hash)
                    .is_ok()
            });

        if !retained_is_valid {
            for path in plan.paths {
                failures.push(DuplicateDeleteFailure {
                    path,
                    reason: "Luna could not verify an unchanged copy to keep, so this file stayed untouched."
                        .to_string(),
                });
            }
            continue;
        }

        for path in plan.paths {
            if let Err(reason) =
                verify_duplicate(&path, plan.group.size_bytes, &plan.group.content_hash)
            {
                failures.push(DuplicateDeleteFailure { path, reason });
                continue;
            }

            match fs::remove_file(&path) {
                Ok(()) => deleted_files.push(DeletedDuplicateFile {
                    path,
                    content_hash: plan.group.content_hash.clone(),
                    size_bytes: plan.group.size_bytes,
                }),
                Err(error) => failures.push(DuplicateDeleteFailure {
                    path,
                    reason: format!("Windows did not remove this file: {error}"),
                }),
            }
        }
    }

    let deleted_paths: HashSet<&str> = deleted_files
        .iter()
        .map(|file| file.path.as_str())
        .collect();
    for group in groups.iter_mut() {
        group
            .files
            .retain(|file| !deleted_paths.contains(file.path.as_str()));
        group.reclaimable_bytes = group
            .size_bytes
            .saturating_mul(group.files.len().saturating_sub(1) as u64);
    }
    groups.retain(|group| group.files.len() > 1);

    Ok(DuplicateDeleteResult {
        removed_bytes: deleted_files
            .iter()
            .fold(0_u64, |total, file| total.saturating_add(file.size_bytes)),
        deleted_files,
        failures,
    })
}

fn validate_request(
    groups: &[DuplicateGroup],
    request: DuplicateDeleteRequest,
) -> Result<Vec<DeletePlan>, String> {
    if request.groups.is_empty() {
        return Err("Select at least one duplicate file to delete.".to_string());
    }

    let requested_count = request
        .groups
        .iter()
        .try_fold(0_usize, |total, group| total.checked_add(group.paths.len()))
        .ok_or_else(|| "The duplicate selection is too large.".to_string())?;
    if requested_count == 0 || requested_count > MAX_DELETE_FILES {
        return Err(format!(
            "Choose between 1 and {MAX_DELETE_FILES} duplicate files at a time."
        ));
    }

    let mut seen_hashes = HashSet::new();
    let mut seen_paths = HashSet::new();
    let mut plans = Vec::new();
    for requested_group in request.groups {
        if !seen_hashes.insert(requested_group.content_hash.clone()) {
            return Err(
                "Each duplicate group may appear only once in a deletion request.".to_string(),
            );
        }
        let group = groups
            .iter()
            .find(|group| group.content_hash == requested_group.content_hash)
            .cloned()
            .ok_or_else(|| {
                "That duplicate group is no longer part of the latest scan. Scan again and retry."
                    .to_string()
            })?;

        let known_paths: HashSet<&str> =
            group.files.iter().map(|file| file.path.as_str()).collect();
        let mut paths = Vec::new();
        for path in requested_group.paths {
            if !known_paths.contains(path.as_str()) {
                return Err(
                    "A selected file is not part of the latest duplicate scan. Scan again and retry."
                        .to_string(),
                );
            }
            if !seen_paths.insert(path.clone()) {
                return Err("A duplicate file may be selected only once.".to_string());
            }
            paths.push(path);
        }
        if paths.is_empty() {
            return Err("Each selected duplicate group must contain a file.".to_string());
        }
        if paths.len() >= group.files.len() {
            return Err("Keep at least one file from every duplicate group.".to_string());
        }
        plans.push(DeletePlan { group, paths });
    }
    Ok(plans)
}

fn verify_duplicate(path: &str, expected_size: u64, expected_hash: &str) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("Luna could not inspect this file again: {error}"))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err("The selected path is no longer a regular file.".to_string());
    }
    if metadata.len() != expected_size {
        return Err("The file size changed after the scan, so Luna left it untouched.".to_string());
    }
    let current_hash = hash_file(Path::new(path))
        .map_err(|error| format!("Luna could not verify the file contents again: {error}"))?;
    if current_hash != expected_hash {
        return Err(
            "The file contents changed after the scan, so Luna left it untouched.".to_string(),
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::DuplicateFile;
    use std::{
        fs,
        time::{SystemTime, UNIX_EPOCH},
    };

    fn test_directory() -> std::path::PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "luna-duplicate-test-{}-{nonce}",
            std::process::id()
        ))
    }

    fn duplicate_group(paths: &[String], hash: String, size: u64) -> DuplicateGroup {
        DuplicateGroup {
            content_hash: hash,
            size_bytes: size,
            reclaimable_bytes: size.saturating_mul(paths.len().saturating_sub(1) as u64),
            files: paths
                .iter()
                .map(|path| DuplicateFile {
                    name: Path::new(path)
                        .file_name()
                        .unwrap()
                        .to_string_lossy()
                        .to_string(),
                    path: path.clone(),
                    last_used_days: Some(1),
                })
                .collect(),
        }
    }

    #[test]
    fn deletes_only_a_verified_selected_copy() {
        let directory = test_directory();
        fs::create_dir_all(&directory).unwrap();
        let keep = directory.join("keep.bin");
        let remove = directory.join("remove.bin");
        fs::write(&keep, b"identical duplicate bytes").unwrap();
        fs::copy(&keep, &remove).unwrap();
        let hash = hash_file(&keep).unwrap();
        let paths = vec![
            keep.to_string_lossy().to_string(),
            remove.to_string_lossy().to_string(),
        ];
        let mut groups = vec![duplicate_group(&paths, hash.clone(), 25)];

        let result = delete_files(
            &mut groups,
            DuplicateDeleteRequest {
                groups: vec![DuplicateDeleteGroup {
                    content_hash: hash,
                    paths: vec![paths[1].clone()],
                }],
            },
        )
        .unwrap();

        assert!(keep.exists());
        assert!(!remove.exists());
        assert_eq!(result.deleted_files.len(), 1);
        assert!(groups.is_empty());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn rejects_deleting_every_copy_in_a_group() {
        let paths = vec!["C:\\keep.bin".to_string(), "C:\\remove.bin".to_string()];
        let mut groups = vec![duplicate_group(&paths, "hash".to_string(), 10)];
        let result = delete_files(
            &mut groups,
            DuplicateDeleteRequest {
                groups: vec![DuplicateDeleteGroup {
                    content_hash: "hash".to_string(),
                    paths,
                }],
            },
        );
        assert!(result.is_err());
    }
}
