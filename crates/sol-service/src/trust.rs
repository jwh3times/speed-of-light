//! Which files the elevated service may trust (ADR 0002). A LocalSystem service must not act
//! on a file a standard user can change, so `install` locks the state folder down and the
//! service checks the startup profile before applying it.

use std::io;
use std::path::Path;

/// SIDs that may own or write files the service trusts: LocalSystem, Administrators and
/// TrustedInstaller.
const TRUSTED_SIDS: &[&str] =
    &["S-1-5-18", "S-1-5-32-544", "S-1-5-80-956008885-3418522649-1831038044-1853292631-2271478464"];

/// Rights that let a holder change what a file says, or replace it in its folder.
const WRITE_MASK: u32 = 0x0000_0002 // FILE_WRITE_DATA / FILE_ADD_FILE
    | 0x0000_0004 // FILE_APPEND_DATA / FILE_ADD_SUBDIRECTORY
    | 0x0000_0040 // FILE_DELETE_CHILD
    | 0x0001_0000 // DELETE
    | 0x0004_0000 // WRITE_DAC
    | 0x0008_0000 // WRITE_OWNER
    | 0x1000_0000 // GENERIC_ALL
    | 0x4000_0000; // GENERIC_WRITE

/// The state folder's ACL: SYSTEM and Administrators full control, Users read and execute,
/// protected so nothing is inherited from `C:\ProgramData` (which lets Users create folders).
pub const STATE_DIR_SDDL: &str = "O:BAD:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;FRFX;;;BU)";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// String SID, e.g. `S-1-5-32-545` for Users.
    pub sid: String,
    pub allow: bool,
    pub mask: u32,
    pub inherited: bool,
    /// Applies only to children, not to this object.
    pub inherit_only: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Acl {
    pub owner: String,
    /// Inheritance from the parent is blocked.
    pub protected: bool,
    pub entries: Vec<Entry>,
}

impl Acl {
    /// SIDs outside SYSTEM, Administrators and TrustedInstaller granted a right to change
    /// this object. Deny entries are ignored, so this errs towards "exposed".
    pub fn non_admin_writers(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for e in &self.entries {
            if e.allow
                && !e.inherit_only
                && e.mask & WRITE_MASK != 0
                && !TRUSTED_SIDS.contains(&e.sid.as_str())
                && !out.contains(&e.sid)
            {
                out.push(e.sid.clone());
            }
        }
        out
    }

    /// An owner can always rewrite the ACL, so only an administrator-side owner is trusted.
    pub fn trusted_owner(&self) -> bool {
        TRUSTED_SIDS.contains(&self.owner.as_str())
    }
}

/// Why the service must not trust `file`, or `None` when only administrators can change it
/// or the folder it sits in.
pub fn untrusted_reason(file: &Path) -> io::Result<Option<String>> {
    let parent = file.parent().filter(|p| !p.as_os_str().is_empty());
    let mut acls = Vec::new();
    for path in std::iter::once(file).chain(parent) {
        acls.push((path, read_acl(path)?));
    }
    Ok(verdict(&acls))
}

/// Why the service must not trust a file, given the ACLs of the file and the folder it sits
/// in; `None` when only administrators can change either.
fn verdict(acls: &[(&Path, Acl)]) -> Option<String> {
    let mut problems = Vec::new();
    for (path, acl) in acls {
        if !acl.trusted_owner() {
            problems.push(format!("{} is owned by {}, not by an administrator", path.display(), acl.owner));
        }
        let writers = acl.non_admin_writers();
        if !writers.is_empty() {
            problems.push(format!(
                "non-administrators can change {} ({})",
                path.display(),
                writers.join(", ")
            ));
        }
    }
    (!problems.is_empty()).then(|| problems.join("; "))
}

/// Give `dir` and everything in it [`STATE_DIR_SDDL`]'s ACL, replacing any explicit entries
/// below it. With `take_ownership` (install, elevated) the owner becomes Administrators too.
pub fn harden_dir(dir: &Path, take_ownership: bool) -> io::Result<()> {
    if take_ownership {
        // A squatter's folder may deny administrators everything. These privileges let an
        // elevated administrator take it back regardless; without them the reset still tries.
        win::enable_privilege("SeTakeOwnershipPrivilege");
        win::enable_privilege("SeRestorePrivilege");
    }
    apply_sddl(dir, STATE_DIR_SDDL, take_ownership, true)
}

use win::apply_sddl;
pub use win::read_acl;

mod win {
    use super::{Acl, Entry};
    use std::ffi::c_void;
    use std::io;
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;
    use std::ptr::NonNull;
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, LUID, LocalFree};
    use windows_sys::Win32::Security::Authorization::{
        ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, GetNamedSecurityInfoW,
        ProgressInvokeNever, SDDL_REVISION_1, SE_FILE_OBJECT, SetNamedSecurityInfoW,
        TreeResetNamedSecurityInfoW,
    };
    use windows_sys::Win32::Security::{
        ACCESS_ALLOWED_ACE, ACE_HEADER, ACL, AdjustTokenPrivileges, DACL_SECURITY_INFORMATION, GetAce,
        GetLengthSid, GetSecurityDescriptorControl, GetSecurityDescriptorDacl, GetSecurityDescriptorOwner,
        INHERIT_ONLY_ACE, INHERITED_ACE, IsValidSid, LUID_AND_ATTRIBUTES, LookupPrivilegeValueW,
        OWNER_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID,
        SE_DACL_PROTECTED, SE_PRIVILEGE_ENABLED, TOKEN_ADJUST_PRIVILEGES, TOKEN_PRIVILEGES, TOKEN_QUERY,
    };
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    const ACCESS_ALLOWED: u8 = 0;
    const ACCESS_DENIED: u8 = 1;

    fn wide(s: &std::ffi::OsStr) -> Vec<u16> {
        s.encode_wide().chain(std::iter::once(0)).collect()
    }

    fn check(err: u32) -> io::Result<()> {
        if err == 0 { Ok(()) } else { Err(io::Error::from_raw_os_error(err as i32)) }
    }

    /// Frees a buffer the system allocated with `LocalAlloc` when dropped.
    struct LocalBox(*mut c_void);
    impl Drop for LocalBox {
        fn drop(&mut self) {
            if !self.0.is_null() {
                // SAFETY: every `LocalBox` wraps a pointer the system allocated for us with
                // LocalAlloc (security descriptors, SID strings), freed exactly once here.
                unsafe { LocalFree(self.0) };
            }
        }
    }

    fn sid_string(sid: PSID) -> io::Result<String> {
        let mut s: *mut u16 = std::ptr::null_mut();
        // SAFETY: `sid` points into a security descriptor that outlives this call; `s` is a
        // valid out-pointer, freed by the `LocalBox`.
        if unsafe { ConvertSidToStringSidW(sid, &mut s) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let _free = LocalBox(s.cast());
        // SAFETY: the system returned a NUL-terminated UTF-16 string at `s`.
        let len = (0..).take_while(|&i| unsafe { *s.add(i) } != 0).count();
        // SAFETY: `len` u16s before the NUL were just read from the same buffer.
        Ok(String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(s, len) }))
    }

    pub fn read_acl(path: &Path) -> io::Result<Acl> {
        let name = wide(path.as_os_str());
        let mut owner: PSID = std::ptr::null_mut();
        let mut dacl: *mut ACL = std::ptr::null_mut();
        let mut sd: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
        // SAFETY: `name` is NUL-terminated; every out-pointer is valid, and `owner` and `dacl`
        // point into `sd`, which the `LocalBox` frees after we're done with them.
        check(unsafe {
            GetNamedSecurityInfoW(
                name.as_ptr(),
                SE_FILE_OBJECT,
                OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
                &mut owner,
                std::ptr::null_mut(),
                &mut dacl,
                std::ptr::null_mut(),
                &mut sd,
            )
        })?;
        let _free = LocalBox(sd);

        let mut control = 0u16;
        let mut revision = 0u32;
        // SAFETY: `sd` is the valid descriptor returned above.
        if unsafe { GetSecurityDescriptorControl(sd, &mut control, &mut revision) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let mut entries = Vec::new();
        match NonNull::new(dacl) {
            None => {
                // A NULL DACL grants everyone everything.
                entries.push(Entry {
                    sid: "S-1-1-0".into(),
                    allow: true,
                    mask: u32::MAX,
                    inherited: false,
                    inherit_only: false,
                });
            }
            Some(dacl) => {
                // SAFETY: `dacl` is non-null and points at the ACL inside `sd`, which `_free`
                // keeps alive until this function returns. The ACL header may be unaligned.
                let count = unsafe { dacl.as_ptr().read_unaligned() }.AceCount;
                for i in 0..u32::from(count) {
                    entries.extend(read_ace(dacl, i)?);
                }
            }
        }
        let owner = NonNull::new(owner).ok_or_else(|| invalid("the file has no owner"))?;
        Ok(Acl { owner: sid_string(owner.as_ptr())?, protected: control & SE_DACL_PROTECTED != 0, entries })
    }

    fn invalid(what: &str) -> io::Error {
        io::Error::new(io::ErrorKind::InvalidData, what.to_string())
    }

    /// Entry `i` of `dacl`, or `None` for ACE types that don't occur on plain files (object and
    /// callback ACEs). Every read is bounds-checked against the ACE's own size.
    fn read_ace(dacl: NonNull<ACL>, i: u32) -> io::Result<Option<Entry>> {
        let mut raw: *mut c_void = std::ptr::null_mut();
        // SAFETY: `dacl` is a valid ACL and `i` is below its AceCount; `raw` is a valid
        // out-pointer.
        if unsafe { GetAce(dacl.as_ptr(), i, &mut raw) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let ace = NonNull::new(raw.cast::<u8>()).ok_or_else(|| invalid("GetAce returned no entry"))?;
        // SAFETY: GetAce succeeded, so `ace` points at an ACE inside the ACL, and every ACE starts
        // with an ACE_HEADER. Read unaligned: nothing promises the header's alignment.
        let header = unsafe { ace.cast::<ACE_HEADER>().as_ptr().read_unaligned() };
        if header.AceType != ACCESS_ALLOWED && header.AceType != ACCESS_DENIED {
            return Ok(None);
        }
        // Allowed and denied ACEs share ACCESS_ALLOWED_ACE's layout: header, mask, then the SID.
        let size = usize::from(header.AceSize);
        let sid_at = std::mem::offset_of!(ACCESS_ALLOWED_ACE, SidStart);
        let mask_at = std::mem::offset_of!(ACCESS_ALLOWED_ACE, Mask);
        if size < sid_at + 8 {
            return Err(invalid("an access entry is too short to hold a SID"));
        }
        // SAFETY: `mask_at + 4 <= sid_at < size`, so the mask lies inside this ACE.
        let mask = unsafe { ace.as_ptr().add(mask_at).cast::<u32>().read_unaligned() };
        // SAFETY: `sid_at < size`, so the SID starts inside this ACE.
        let sid: PSID = unsafe { ace.as_ptr().add(sid_at) }.cast();
        // SAFETY: `sid` points inside the ACE; IsValidSid and GetLengthSid only read the SID's
        // revision and sub-authority count before the length is checked against the ACE.
        let fits = unsafe { IsValidSid(sid) != 0 && GetLengthSid(sid) as usize <= size - sid_at };
        if !fits {
            return Err(invalid("an access entry holds an invalid SID"));
        }
        Ok(Some(Entry {
            sid: sid_string(sid)?,
            allow: header.AceType == ACCESS_ALLOWED,
            mask,
            inherited: u32::from(header.AceFlags) & INHERITED_ACE != 0,
            inherit_only: u32::from(header.AceFlags) & INHERIT_ONLY_ACE != 0,
        }))
    }

    /// Set `path`'s DACL (always protected) from `sddl`, and its owner too with `set_owner`.
    /// With `tree`, every object below `path` is reset to inherit it, dropping explicit entries.
    pub fn apply_sddl(path: &Path, sddl: &str, set_owner: bool, tree: bool) -> io::Result<()> {
        let sddl = wide(std::ffi::OsStr::new(sddl));
        let mut sd: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
        // SAFETY: `sddl` is NUL-terminated UTF-16 and `sd` a valid out-pointer, freed below.
        if unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                SDDL_REVISION_1,
                &mut sd,
                std::ptr::null_mut(),
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        let _free = LocalBox(sd);
        let (mut present, mut defaulted) = (0, 0);
        let mut dacl: *mut ACL = std::ptr::null_mut();
        let mut owner: PSID = std::ptr::null_mut();
        // SAFETY: `sd` is valid; the out-pointers point into it and live as long as `_free`.
        unsafe {
            if GetSecurityDescriptorDacl(sd, &mut present, &mut dacl, &mut defaulted) == 0 {
                return Err(io::Error::last_os_error());
            }
            if set_owner && GetSecurityDescriptorOwner(sd, &mut owner, &mut defaulted) == 0 {
                return Err(io::Error::last_os_error());
            }
        }
        if present == 0 || dacl.is_null() || (set_owner && owner.is_null()) {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "SDDL lacks a DACL or owner"));
        }
        let mut info = DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION;
        if set_owner {
            info |= OWNER_SECURITY_INFORMATION;
        } else {
            owner = std::ptr::null_mut();
        }
        let name = wide(path.as_os_str());
        // SAFETY: `name` is NUL-terminated; `owner` and `dacl` point into `sd`, still alive.
        check(unsafe {
            if tree {
                TreeResetNamedSecurityInfoW(
                    name.as_ptr(),
                    SE_FILE_OBJECT,
                    info,
                    owner,
                    std::ptr::null_mut(),
                    dacl,
                    std::ptr::null(),
                    0,
                    None,
                    ProgressInvokeNever,
                    std::ptr::null(),
                )
            } else {
                SetNamedSecurityInfoW(
                    name.as_ptr(),
                    SE_FILE_OBJECT,
                    info,
                    owner,
                    std::ptr::null_mut(),
                    dacl,
                    std::ptr::null(),
                )
            }
        })
    }

    /// Best effort: enable `name` in this process's token if the user holds it.
    pub fn enable_privilege(name: &str) {
        let name = wide(std::ffi::OsStr::new(name));
        let mut token: HANDLE = std::ptr::null_mut();
        // SAFETY: GetCurrentProcess returns a pseudo-handle; `token` is a valid out-pointer and
        // is closed below. `tp` is a fully initialised TOKEN_PRIVILEGES with one entry.
        unsafe {
            if OpenProcessToken(GetCurrentProcess(), TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY, &mut token) == 0 {
                return;
            }
            let mut luid = LUID { LowPart: 0, HighPart: 0 };
            if LookupPrivilegeValueW(std::ptr::null(), name.as_ptr(), &mut luid) != 0 {
                let tp = TOKEN_PRIVILEGES {
                    PrivilegeCount: 1,
                    Privileges: [LUID_AND_ATTRIBUTES { Luid: luid, Attributes: SE_PRIVILEGE_ENABLED }],
                };
                AdjustTokenPrivileges(token, 0, &tp, 0, std::ptr::null_mut(), std::ptr::null_mut());
            }
            CloseHandle(token);
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    const USERS: &str = "S-1-5-32-545";

    /// An ACL a test sets itself; `harden_dir`'s owner change needs elevation, so tests that
    /// run unelevated never set the owner.
    fn set_dacl(path: &Path, dacl_sddl: &str) {
        apply_sddl(path, dacl_sddl, false, false).unwrap();
    }

    #[test]
    fn a_file_users_can_write_is_exposed() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("startup-profile.toml");
        std::fs::write(&file, "schema = 1\n").unwrap();
        set_dacl(&file, "D:P(A;;FA;;;SY)(A;;FA;;;BA)(A;;FA;;;BU)");
        assert_eq!(read_acl(&file).unwrap().non_admin_writers(), vec![USERS.to_string()]);
        let why = untrusted_reason(&file).unwrap().unwrap();
        assert!(why.contains(USERS), "{why}");
    }

    #[test]
    fn a_file_users_can_only_read_has_no_non_admin_writers() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("startup-profile.toml");
        std::fs::write(&file, "schema = 1\n").unwrap();
        set_dacl(&file, "D:P(A;;FA;;;SY)(A;;FA;;;BA)(A;;FR;;;BU)");
        assert!(read_acl(&file).unwrap().non_admin_writers().is_empty());
    }

    #[test]
    fn a_file_owned_by_a_standard_user_is_not_trusted() {
        // Built by hand: who owns a file a test creates depends on whether the test runs
        // elevated (CI does), and an owner can always rewrite the ACL.
        let owned_by = |owner: &str| Acl { owner: owner.into(), protected: true, entries: Vec::new() };
        assert!(!owned_by("S-1-5-21-1004336348-1177238915-682003330-1001").trusted_owner());
        assert!(!owned_by(USERS).trusted_owner());
        assert!(owned_by("S-1-5-32-544").trusted_owner(), "Administrators");
        assert!(owned_by("S-1-5-18").trusted_owner(), "LocalSystem");
        assert!(
            owned_by("S-1-5-80-956008885-3418522649-1831038044-1853292631-2271478464").trusted_owner(),
            "TrustedInstaller"
        );
    }

    fn entry(sid: &str, mask: u32) -> Entry {
        Entry { sid: sid.into(), allow: true, mask, inherited: false, inherit_only: false }
    }

    /// SYSTEM and Administrators full control, Users read: what `install` leaves behind.
    fn locked_down(owner: &str) -> Acl {
        Acl {
            owner: owner.into(),
            protected: true,
            entries: vec![
                entry("S-1-5-18", 0x1F_01FF),
                entry("S-1-5-32-544", 0x1F_01FF),
                entry(USERS, 0x12_00A9),
            ],
        }
    }

    #[test]
    fn a_profile_only_administrators_can_change_is_trusted() {
        let (file, folder) = (
            Path::new(r"C:\ProgramData\SpeedOfLight\startup-profile.toml"),
            Path::new(r"C:\ProgramData\SpeedOfLight"),
        );
        assert_eq!(verdict(&[(file, locked_down("S-1-5-32-544")), (folder, locked_down("S-1-5-18"))]), None);
    }

    #[test]
    fn a_profile_owned_by_a_standard_user_is_not_trusted_even_if_its_entries_are_locked_down() {
        let user = "S-1-5-21-1004336348-1177238915-682003330-1001";
        let (file, folder) = (
            Path::new(r"C:\ProgramData\SpeedOfLight\startup-profile.toml"),
            Path::new(r"C:\ProgramData\SpeedOfLight"),
        );
        let why = verdict(&[(file, locked_down(user)), (folder, locked_down("S-1-5-32-544"))]).unwrap();
        assert!(why.contains(&format!("{} is owned by {user}", file.display())), "{why}");
        let why = verdict(&[(file, locked_down("S-1-5-32-544")), (folder, locked_down(user))]).unwrap();
        assert!(why.contains(&format!("{} is owned by {user}", folder.display())), "{why}");
    }

    #[test]
    fn a_writer_with_several_entries_is_named_once() {
        let acl = Acl {
            owner: "S-1-5-32-544".into(),
            protected: true,
            entries: vec![entry(USERS, 0x1F_01FF), entry(USERS, 0x12_0116), entry("S-1-5-18", 0x1F_01FF)],
        };
        assert_eq!(acl.non_admin_writers(), vec![USERS.to_string()]);
    }

    #[test]
    fn an_entry_that_only_applies_to_children_does_not_expose_the_folder_itself() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("state");
        std::fs::create_dir(&folder).unwrap();
        set_dacl(&folder, "D:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICIIO;FA;;;BU)");
        assert!(read_acl(&folder).unwrap().non_admin_writers().is_empty());
    }

    #[test]
    fn a_deny_entry_grants_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("startup-profile.toml");
        std::fs::write(&file, "schema = 1\n").unwrap();
        set_dacl(&file, "D:P(D;;FA;;;BU)(A;;FA;;;SY)(A;;FA;;;BA)");
        assert!(read_acl(&file).unwrap().non_admin_writers().is_empty());
    }

    #[test]
    fn a_file_in_a_folder_users_can_write_is_exposed_even_if_the_file_is_locked_down() {
        // Whoever can write the folder can delete the file and put their own in its place.
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("state");
        std::fs::create_dir(&folder).unwrap();
        let file = folder.join("startup-profile.toml");
        std::fs::write(&file, "schema = 1\n").unwrap();
        set_dacl(&file, "D:P(A;;FA;;;SY)(A;;FA;;;BA)(A;;FR;;;BU)");
        set_dacl(&folder, "D:P(A;;FA;;;SY)(A;;FA;;;BA)(A;;FA;;;BU)");
        let why = untrusted_reason(&file).unwrap().unwrap();
        assert!(why.contains(&format!("non-administrators can change {}", folder.display())), "{why}");
        assert!(!why.contains(&format!("non-administrators can change {}", file.display())), "{why}");
    }

    #[test]
    fn hardening_a_folder_protects_it_and_clears_explicit_entries_below_it() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("SpeedOfLight");
        std::fs::create_dir(&state).unwrap();
        let planted = state.join("crash-guard.json");
        std::fs::write(&planted, "{}").unwrap();
        // A squatter's folder: Users may do anything, in the folder and to what's in it.
        set_dacl(&state, "D:P(A;OICI;FA;;;BU)(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)");
        set_dacl(&planted, "D:P(A;;FA;;;BU)");

        harden_dir(&state, false).unwrap();

        let acl = read_acl(&state).unwrap();
        assert!(acl.protected);
        assert!(acl.entries.iter().all(|e| !e.inherited), "{acl:?}");
        assert!(acl.non_admin_writers().is_empty(), "{acl:?}");
        assert!(acl.entries.iter().any(|e| e.sid == USERS && e.allow && e.mask & 0x1 != 0), "Users can read");
        let child = read_acl(&planted).unwrap();
        assert!(child.non_admin_writers().is_empty(), "{child:?}");
        assert!(child.entries.iter().all(|e| e.inherited), "explicit entries below are replaced: {child:?}");
    }
}
