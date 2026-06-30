//! NoCloud cloud-init seed: a small FAT image labelled CIDATA holding
//! `user-data` (raw #cloud-config) and `meta-data`. Pure Rust (fatfs).

use std::io::{Seek, SeekFrom, Write};
use std::path::Path;

use fscommon::BufStream;

const SEED_SIZE: u64 = 1024 * 1024; // 1 MiB is ample for cloud-init seeds

pub fn write_seed_img(
    path: &Path,
    instance_id: &str,
    hostname: &str,
    user_data: &str,
) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let img = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)?;
    img.set_len(SEED_SIZE)?;

    let mut buf_stream = BufStream::new(img);
    fatfs::format_volume(
        &mut buf_stream,
        fatfs::FormatVolumeOptions::new().volume_label(*b"CIDATA     "),
    )?;
    buf_stream.seek(SeekFrom::Start(0))?;

    let fs = fatfs::FileSystem::new(buf_stream, fatfs::FsOptions::new())?;
    {
        let root = fs.root_dir();
        let meta = format!("instance-id: {instance_id}\nlocal-hostname: {hostname}\n");
        let mut f = root.create_file("meta-data")?;
        f.write_all(meta.as_bytes())?;
        let mut u = root.create_file("user-data")?;
        u.write_all(user_data.as_bytes())?;
    }
    fs.unmount()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    #[test]
    fn seed_roundtrips_files_and_label() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("seed.img");
        write_seed_img(&path, "vm-123", "web1", "#cloud-config\nhostname: web1\n").unwrap();

        let img = std::fs::OpenOptions::new().read(true).write(true).open(&path).unwrap();
        let buf_stream = BufStream::new(img);
        let fs = fatfs::FileSystem::new(buf_stream, fatfs::FsOptions::new()).unwrap();
        let label = fs.volume_label();
        assert_eq!(label.trim(), "CIDATA");
        let root = fs.root_dir();
        let mut s = String::new();
        root.open_file("user-data").unwrap().read_to_string(&mut s).unwrap();
        assert!(s.contains("#cloud-config"));
        let mut m = String::new();
        root.open_file("meta-data").unwrap().read_to_string(&mut m).unwrap();
        assert!(m.contains("instance-id: vm-123"));
        assert!(m.contains("local-hostname: web1"));
    }
}
