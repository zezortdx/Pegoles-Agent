//! The one file a VM may use: the user's own computer disk, reached
//! without any junction, symbolic link or other reparse point, and
//! opened with the user's rights.

use windows::Win32::Foundation::GENERIC_READ;
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FileAttributeTagInfo, GetFileInformationByHandleEx, FILE_ATTRIBUTE_REPARSE_POINT,
    FILE_ATTRIBUTE_TAG_INFO, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
    FILE_GENERIC_READ, FILE_GENERIC_WRITE, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    OPEN_EXISTING,
};

use super::client::Client;
use super::util::{pcwstr, wide, Owned};

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

fn open_no_follow(path: &str, access: u32) -> Result<Owned, String> {
    let name = wide(path);
    // SAFETY: NUL-terminated path; OPEN_REPARSE_POINT opens a link itself
    // instead of its target, so the check below sees it.
    unsafe {
        CreateFileW(
            pcwstr(&name),
            access,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            None,
            OPEN_EXISTING,
            FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS,
            None,
        )
    }
    .map(Owned)
    .map_err(|e| format!("cannot open {path}: {e}"))
}

/// Check, as the user, that `<LocalAppData>\Pegoles\computers\<id>\disk.vhdx`
/// exists, that no component from `Pegoles` down is a reparse point, and
/// that the user can read and write the disk. Nothing is followed.
pub fn check_disk(client: &Client, computer_id: &str, disk: &str) -> Result<(), String> {
    let root = client.local_app_data.trim_end_matches(['\\', '/']);
    let components = [
        format!(r"{root}\Pegoles"),
        format!(r"{root}\Pegoles\computers"),
        format!(r"{root}\Pegoles\computers\{computer_id}"),
    ];
    client.as_user(|| {
        for dir in &components {
            let handle = open_no_follow(dir, GENERIC_READ.0)?;
            if is_reparse_point(&handle)? {
                return Err(format!("{dir} is a link; Pegoles refuses linked folders"));
            }
        }
        let handle = open_no_follow(disk, FILE_GENERIC_READ.0 | FILE_GENERIC_WRITE.0)?;
        if is_reparse_point(&handle)? {
            return Err("the disk is a link; Pegoles refuses linked files".into());
        }
        Ok(())
    })
}
