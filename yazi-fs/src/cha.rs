use std::{fs::{FileType, Metadata}, path::Path, time::SystemTime};

use bitflags::bitflags;
use yazi_macro::{unix_either, win_either};

bitflags! {
	#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
	pub struct ChaKind: u8 {
		const DIR    = 0b00000001;

		const HIDDEN = 0b00000010;
		const LINK   = 0b00000100;
		const ORPHAN = 0b00001000;

		const DUMMY  = 0b00010000;
		#[cfg(windows)]
		const SYSTEM = 0b00100000;
	}
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Cha {
	pub kind:  ChaKind,
	pub len:   u64,
	pub atime: Option<SystemTime>,
	pub btime: Option<SystemTime>,
	#[cfg(unix)]
	pub ctime: Option<SystemTime>,
	pub mtime: Option<SystemTime>,
	#[cfg(unix)]
	pub mode:  libc::mode_t,
	#[cfg(unix)]
	pub dev:   libc::dev_t,
	#[cfg(unix)]
	pub uid:   libc::uid_t,
	#[cfg(unix)]
	pub gid:   libc::gid_t,
	#[cfg(unix)]
	pub nlink: libc::nlink_t,
}

impl From<Metadata> for Cha {
	fn from(m: Metadata) -> Self {
		let mut kind = ChaKind::empty();
		if m.is_dir() {
			kind |= ChaKind::DIR;
		} else if m.is_symlink() {
			kind |= ChaKind::LINK;
		}

		Self {
			kind,
			len: m.len(),
			atime: m.accessed().ok(),
			btime: m.created().ok(),
			#[cfg(unix)]
			ctime: {
				use std::{os::unix::fs::MetadataExt, time::{Duration, UNIX_EPOCH}};
				UNIX_EPOCH.checked_add(Duration::new(m.ctime() as u64, m.ctime_nsec() as u32))
			},
			mtime: m.modified().ok(),

			#[cfg(unix)]
			mode: {
				use std::os::unix::prelude::PermissionsExt;
				m.permissions().mode() as _
			},
			#[cfg(unix)]
			dev: {
				use std::os::unix::fs::MetadataExt;
				m.dev() as _
			},
			#[cfg(unix)]
			uid: {
				use std::os::unix::fs::MetadataExt;
				m.uid() as _
			},
			#[cfg(unix)]
			gid: {
				use std::os::unix::fs::MetadataExt;
				m.gid() as _
			},
			#[cfg(unix)]
			nlink: {
				use std::os::unix::fs::MetadataExt;
				m.nlink() as _
			},
		}
	}
}

impl From<FileType> for Cha {
	fn from(t: FileType) -> Self {
		let mut kind = ChaKind::DUMMY;

		#[cfg(unix)]
		let mode = {
			use std::os::unix::fs::FileTypeExt;
			if t.is_dir() {
				kind |= ChaKind::DIR;
				libc::S_IFDIR
			} else if t.is_symlink() {
				kind |= ChaKind::LINK;
				libc::S_IFLNK
			} else if t.is_block_device() {
				libc::S_IFBLK
			} else if t.is_char_device() {
				libc::S_IFCHR
			} else if t.is_fifo() {
				libc::S_IFIFO
			} else if t.is_socket() {
				libc::S_IFSOCK
			} else {
				0
			}
		};

		#[cfg(windows)]
		{
			if t.is_dir() {
				kind |= ChaKind::DIR;
			} else if t.is_symlink() {
				kind |= ChaKind::LINK;
			}
		}

		Self {
			kind,
			#[cfg(unix)]
			mode,
			..Default::default()
		}
	}
}

impl Cha {
	#[inline]
	pub async fn new(path: &Path, mut meta: Metadata) -> Self {
		let mut attached = ChaKind::empty();

		if meta.is_symlink() {
			attached |= ChaKind::LINK;
			meta = tokio::fs::metadata(path).await.unwrap_or(meta);
		}
		if meta.is_symlink() {
			attached |= ChaKind::ORPHAN;
		}

		let mut cha = Self::new_nofollow(path, meta);
		cha.kind |= attached;
		cha
	}

	#[inline]
	pub fn new_nofollow(_path: &Path, meta: Metadata) -> Self {
		let mut attached = ChaKind::empty();

		#[cfg(unix)]
		if yazi_shared::url::Urn::new(_path).is_hidden() {
			attached |= ChaKind::HIDDEN;
		}
		#[cfg(windows)]
		{
			use std::os::windows::fs::MetadataExt;

			use windows_sys::Win32::Storage::FileSystem::{FILE_ATTRIBUTE_HIDDEN, FILE_ATTRIBUTE_SYSTEM};
			if meta.file_attributes() & FILE_ATTRIBUTE_HIDDEN != 0 {
				attached |= ChaKind::HIDDEN;
			}
			if meta.file_attributes() & FILE_ATTRIBUTE_SYSTEM != 0 {
				attached |= ChaKind::SYSTEM;
			}
		}

		let mut cha = Self::from(meta);
		cha.kind |= attached;
		cha
	}

	#[inline]
	pub fn dummy() -> Self { Self { kind: ChaKind::DUMMY, ..Default::default() } }

	#[inline]
	pub fn hits(self, c: Self) -> bool {
		self.len == c.len
			&& self.mtime == c.mtime
			&& unix_either!(self.ctime == c.ctime, true)
			&& self.btime == c.btime
			&& self.kind == c.kind
			&& unix_either!(self.mode == c.mode, true)
	}
}

impl Cha {
	#[inline]
	pub const fn is_dir(&self) -> bool { self.kind.contains(ChaKind::DIR) }

	#[inline]
	pub const fn is_hidden(&self) -> bool {
		self.kind.contains(ChaKind::HIDDEN) || win_either!(self.kind.contains(ChaKind::SYSTEM), false)
	}

	#[inline]
	pub const fn is_link(&self) -> bool { self.kind.contains(ChaKind::LINK) }

	#[inline]
	pub const fn is_orphan(&self) -> bool { self.kind.contains(ChaKind::ORPHAN) }

	#[inline]
	pub const fn is_dummy(&self) -> bool { self.kind.contains(ChaKind::DUMMY) }

	#[inline]
	pub const fn is_block(&self) -> bool {
		unix_either!(self.mode & libc::S_IFMT == libc::S_IFBLK, false)
	}

	#[inline]
	pub const fn is_char(&self) -> bool {
		unix_either!(self.mode & libc::S_IFMT == libc::S_IFCHR, false)
	}

	#[inline]
	pub const fn is_fifo(&self) -> bool {
		unix_either!(self.mode & libc::S_IFMT == libc::S_IFIFO, false)
	}

	#[inline]
	pub const fn is_sock(&self) -> bool {
		unix_either!(self.mode & libc::S_IFMT == libc::S_IFSOCK, false)
	}

	#[inline]
	pub const fn is_exec(&self) -> bool { unix_either!(self.mode & libc::S_IXUSR != 0, false) }

	#[inline]
	pub const fn is_sticky(&self) -> bool { unix_either!(self.mode & libc::S_ISVTX != 0, false) }
}
