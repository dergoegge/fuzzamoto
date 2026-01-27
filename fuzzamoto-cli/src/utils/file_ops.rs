use crate::error::{CliError, Result};
use std::fs;
use std::path::Path;

pub fn copy_file_to_dir(src: &Path, dst_dir: &Path) -> Result<()> {
    let file_name = src
        .file_name()
        .ok_or_else(|| CliError::InvalidInput("Invalid source file path".to_string()))?;

    let dst_path = dst_dir.join(file_name);
    fs::copy(src, &dst_path)?;

    log::info!("Copied file: {} -> {}", src.display(), dst_path.display());
    Ok(())
}

pub fn ensure_file_exists(path: &Path) -> Result<()> {
    if !path.exists() || !path.is_file() {
        return Err(CliError::FileNotFound(path.display().to_string()));
    }
    Ok(())
}

pub fn ensure_dir_exists(path: &Path) -> Result<()> {
    if !path.exists() || !path.is_dir() {
        return Err(CliError::FileNotFound(path.display().to_string()));
    }
    Ok(())
}

pub fn ensure_sharedir_not_exists(path: &Path) -> Result<()> {
    if path.exists() {
        return Err(CliError::ShareDirExists);
    }
    Ok(())
}

pub fn create_dir_all(path: &Path) -> Result<()> {
    fs::create_dir_all(path)?;
    Ok(())
}

pub fn read_dir_files(dir: &Path) -> Result<Vec<std::path::PathBuf>> {
    let mut files = Vec::new();
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_file() && !path.file_name().unwrap().to_str().unwrap().starts_with('.') {
            files.push(path);
        }
    }
    Ok(files)
}

pub fn copy_dir_contents(src: &Path, dst: &Path) -> Result<()> {
    for entry in fs::read_dir(src)? {
        let path = entry?.path();
        if path.is_file() {
            let dst_path = dst.join(path.file_name().unwrap());
            fs::copy(&path, &dst_path)?;
            log::info!("Copied: {} -> {}", path.display(), dst_path.display());
        }
    }
    Ok(())
}
