//! The one file a VM may use: the user's own computer disk, reached
//! without any junction, symbolic link or other reparse point, and
//! opened with the user's rights.
//!
//! Checking a path and handing the same path to HCS later is a race: the
//! user owns these folders and could swap one for a junction in between,
//! or remap a drive letter in their own session so that the check sees
//! one tree and the service another. So the check returns a [`DiskLease`]
//! that keeps every folder open without delete sharing for as long as the
//! VM exists (a folder that is open cannot be renamed, removed or turned
//! into a junction), and each folder and the disk must be the same file
//! object whether resolved as the user or as the service.

use windows::Win32::Foundation::GENERIC_READ;
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FileAttributeTagInfo, FileIdInfo, GetFileInformationByHandleEx,
    FILE_ATTRIBUTE_REPARSE_POINT, FILE_ATTRIBUTE_TAG_INFO, FILE_FLAG_BACKUP_SEMANTICS,
    FILE_FLAG_OPEN_REPARSE_POINT, FILE_GENERIC_READ, FILE_GENERIC_WRITE, FILE_ID_INFO,
    FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_MODE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    OPEN_EXISTING,
};

use super::client::Client;
use super::util::{pcwstr, wide, Owned};

/// Open folder handles that pin a computer's path while its VM exists.
pub struct DiskLease {
    _held: Vec<Owned>,
}

/// Volume serial number + 128-bit file id: one file object.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct FileId(u64, [u8; 16]);

fn is_reparse_point(handle: &Owned) -> Result<bool, String> {
    let mut info = FILE_ATTRIBUTE_TAG_INFO::default();
    // SAFETY: correctly sized out-parameter for this class.
    unsafe {
        GetFileInformationByHandleEx(
            handle.raw(),
            FileAttributeTagInfo,
            (&mut info as *mut FILE_ATTRIBUTE_TAG_INFO).cast(),
            std::mem::size_of::<FILE_ATTRIBUTE_TAG_INFO>() as u32,
        )
    }
    .map_err(|e| format!("cannot inspect a path: {e}"))?;
    Ok(info.FileAttributes & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0 || info.ReparseTag != 0)
}

fn file_id(handle: &Owned) -> Result<FileId, String> {
    let mut info = FILE_ID_INFO::default();
    // SAFETY: correctly sized out-parameter for this class.
    unsafe {
        GetFileInformationByHandleEx(
            handle.raw(),
            FileIdInfo,
            (&mut info as *mut FILE_ID_INFO).cast(),
            std::mem::size_of::<FILE_ID_INFO>() as u32,
        )
    }
    .map_err(|e| format!("cannot identify a path: {e}"))?;
    Ok(FileId(info.VolumeSerialNumber, info.FileId.Identifier))
}

fn open_no_follow(path: &str, access: u32, share: FILE_SHARE_MODE) -> Result<Owned, String> {
    let name = wide(path);
    // SAFETY: NUL-terminated path; OPEN_REPARSE_POINT opens a link itself
    // instead of its target, so the checks below see it.
    unsafe {
        CreateFileW(
            pcwstr(&name),
            access,
            share,
            None,
            OPEN_EXISTING,
            FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS,
            None,
        )
    }
    .map(Owned)
    .map_err(|e| format!("cannot open {path}: {e}"))
}

/// Open a folder so that it cannot be renamed or removed while held (no
/// delete sharing on a handle with data access), refuse links, identify it.
fn pin_folder(path: &str) -> Result<(Owned, FileId), String> {
    let handle = open_no_follow(path, GENERIC_READ.0, FILE_SHARE_READ | FILE_SHARE_WRITE)?;
    if is_reparse_point(&handle)? {
        return Err(format!("{path} is a link; Pegoles refuses linked folders"));
    }
    let id = file_id(&handle)?;
    Ok((handle, id))
}

/// Check `<LocalAppData>\Pegoles\computers\<id>\disk.vhdx`: no component
/// from `Pegoles` down is a reparse point, the user can read and write the
/// disk, and the user and the service resolve every component to the same
/// file object. The folders stay pinned by the returned lease.
pub fn check_disk(client: &Client, computer_id: &str, disk: &str) -> Result<DiskLease, String> {
    let root = client.local_app_data.trim_end_matches(['\\', '/']);
    let folders = [
        format!(r"{root}\Pegoles"),
        format!(r"{root}\Pegoles\computers"),
        format!(r"{root}\Pegoles\computers\{computer_id}"),
    ];
    // As the user: their rights and their drive letters.
    let (mut held, user_ids, user_disk) = client.as_user(|| {
        let mut held = Vec::new();
        let mut ids = Vec::new();
        for folder in &folders {
            let (handle, id) = pin_folder(folder)?;
            held.push(handle);
            ids.push(id);
        }
        let rw = open_no_follow(
            disk,
            FILE_GENERIC_READ.0 | FILE_GENERIC_WRITE.0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
        )?;
        if is_reparse_point(&rw)? {
            return Err("the disk is a link; Pegoles refuses linked files".to_string());
        }
        let disk_id = file_id(&rw)?;
        Ok((held, ids, disk_id))
    })?;
    // As the service: what HCS will resolve. Same objects, or refuse.
    for (folder, user_id) in folders.iter().zip(&user_ids) {
        let (handle, id) = pin_folder(folder)?;
        if id != *user_id {
            return Err(format!(
                "{folder} is not the same folder for Pegoles' service as for you"
            ));
        }
        held.push(handle);
    }
    let service_disk = open_no_follow(
        disk,
        FILE_READ_ATTRIBUTES.0,
        FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
    )?;
    if is_reparse_point(&service_disk)? || file_id(&service_disk)? != user_disk {
        return Err("the disk is not the same file for Pegoles' service as for you".into());
    }
    Ok(DiskLease { _held: held })
}
