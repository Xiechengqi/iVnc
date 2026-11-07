//! System call filters. Docker's default seccomp profile has to be lifted so that
//! bubblewrap can create namespaces and mount, which the iVnc image restores
//! without a profile file: the manager filters itself and everything it starts
//! (`Profile::Container`). Once a strict sandbox exists, its worker also removes
//! namespaces and mounts, from itself and from the application
//! (`Profile::Application`). A filter is inherited and can never be lifted.

/// What a filter denies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Profile {
    /// Kernel-wide and frequently exploited interfaces, as Docker's default
    /// profile denies them, for the manager in a container.
    Container,
    /// Additionally namespaces and mounts, for strict application runtimes.
    Application,
}

#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
mod imp {
    use super::Profile;
    use libc::{
        seccomp_data, sock_filter, BPF_ABS, BPF_JEQ, BPF_JGE, BPF_JMP, BPF_JSET, BPF_K, BPF_LD,
        BPF_RET, BPF_W, SECCOMP_RET_ALLOW, SECCOMP_RET_ERRNO, SECCOMP_RET_KILL_PROCESS,
    };
    use std::mem::offset_of;

    #[cfg(target_arch = "x86_64")]
    const AUDIT_ARCH: u32 = 0xc000_003e;
    #[cfg(target_arch = "aarch64")]
    const AUDIT_ARCH: u32 = 0xc000_00b7;

    /// clone flags that create namespaces.
    const NAMESPACE_FLAGS: u32 = (libc::CLONE_NEWNS
        | libc::CLONE_NEWCGROUP
        | libc::CLONE_NEWUTS
        | libc::CLONE_NEWIPC
        | libc::CLONE_NEWUSER
        | libc::CLONE_NEWPID
        | libc::CLONE_NEWNET) as u32;

    /// Nested namespaces would hand the application administrator-only kernel
    /// code (nf_tables, mounts) inside them.
    const NAMESPACES: &[libc::c_long] = &[
        libc::SYS_unshare,
        libc::SYS_setns,
        libc::SYS_mount,
        libc::SYS_umount2,
        libc::SYS_pivot_root,
        libc::SYS_open_tree,
        libc::SYS_move_mount,
        libc::SYS_fsopen,
        libc::SYS_fsconfig,
        libc::SYS_fsmount,
        libc::SYS_fspick,
        libc::SYS_mount_setattr,
    ];
    /// What the former Docker profile of the image denied.
    const KERNEL: &[libc::c_long] = &[
        // Large or frequently exploited interfaces desktop software does not use.
        libc::SYS_io_uring_setup,
        libc::SYS_io_uring_enter,
        libc::SYS_io_uring_register,
        libc::SYS_bpf,
        libc::SYS_userfaultfd,
        libc::SYS_perf_event_open,
        libc::SYS_keyctl,
        libc::SYS_add_key,
        libc::SYS_request_key,
        libc::SYS_open_by_handle_at,
        // Host administration.
        libc::SYS_kexec_load,
        libc::SYS_kexec_file_load,
        libc::SYS_init_module,
        libc::SYS_finit_module,
        libc::SYS_delete_module,
        libc::SYS_acct,
        libc::SYS_swapon,
        libc::SYS_swapoff,
        libc::SYS_reboot,
        libc::SYS_quotactl,
        libc::SYS_quotactl_fd,
        libc::SYS_lookup_dcookie,
        libc::SYS_settimeofday,
        libc::SYS_clock_settime,
        libc::SYS_clock_adjtime,
        libc::SYS_nfsservctl,
    ];
    #[cfg(target_arch = "x86_64")]
    const KERNEL_ARCH: &[libc::c_long] = &[
        libc::SYS_ioperm,
        libc::SYS_iopl,
        libc::SYS_uselib,
        libc::SYS_ustat,
        libc::SYS_sysfs,
        libc::SYS__sysctl,
    ];
    #[cfg(target_arch = "aarch64")]
    const KERNEL_ARCH: &[libc::c_long] = &[];
    /// Not needed by applications, though an administrator's shell may use them.
    #[cfg(target_arch = "x86_64")]
    const APPLICATION_EXTRA: &[libc::c_long] = &[libc::SYS_syslog, libc::SYS_modify_ldt];
    #[cfg(target_arch = "aarch64")]
    const APPLICATION_EXTRA: &[libc::c_long] = &[libc::SYS_syslog];

    // libc's BPF_STMT and BPF_JUMP are needlessly unsafe functions.
    fn instruction(code: u32, k: u32, jt: u8, jf: u8) -> sock_filter {
        sock_filter {
            code: code as u16,
            jt,
            jf,
            k,
        }
    }
    fn load(offset: usize) -> sock_filter {
        instruction(BPF_LD | BPF_W | BPF_ABS, offset as u32, 0, 0)
    }
    fn jump(condition: u32, k: u32, jt: u8, jf: u8) -> sock_filter {
        instruction(BPF_JMP | condition | BPF_K, k, jt, jf)
    }
    fn ret(action: u32) -> sock_filter {
        instruction(BPF_RET | BPF_K, action, 0, 0)
    }

    /// Builds the filter. Every comparison is directly followed by its action, so
    /// all jumps are short.
    pub fn program(profile: Profile) -> Vec<sock_filter> {
        let deny = SECCOMP_RET_ERRNO | libc::EPERM as u32;
        let mut filter = vec![
            load(offset_of!(seccomp_data, arch)),
            // Another ABI (32-bit `int 0x80`) numbers system calls differently,
            // so none of the checks below would apply to it.
            jump(BPF_JEQ, AUDIT_ARCH, 1, 0),
            ret(SECCOMP_RET_KILL_PROCESS),
            load(offset_of!(seccomp_data, nr)),
        ];
        // x32 calls report the x86_64 architecture but use their own numbers.
        #[cfg(target_arch = "x86_64")]
        filter.extend([jump(BPF_JGE, 0x4000_0000, 0, 1), ret(deny)]);
        let mut denied: Vec<libc::c_long> = KERNEL.iter().chain(KERNEL_ARCH).copied().collect();
        if profile == Profile::Application {
            filter.extend([
                // clone3 passes its flags in memory a filter cannot read. Like
                // Docker's default profile, report it missing so libc falls back
                // to clone.
                jump(BPF_JEQ, libc::SYS_clone3 as u32, 0, 1),
                ret(SECCOMP_RET_ERRNO | libc::ENOSYS as u32),
                jump(BPF_JEQ, libc::SYS_clone as u32, 0, 4),
                // The flags are the first argument; namespace flags fit its low 32 bits.
                load(offset_of!(seccomp_data, args)),
                jump(BPF_JSET, NAMESPACE_FLAGS, 0, 1),
                ret(deny),
                ret(SECCOMP_RET_ALLOW),
            ]);
            denied.extend(NAMESPACES.iter().chain(APPLICATION_EXTRA));
        }
        for nr in denied {
            filter.extend([jump(BPF_JEQ, nr as u32, 0, 1), ret(deny)]);
        }
        filter.push(ret(SECCOMP_RET_ALLOW));
        filter
    }

    /// Installs the filter for the calling process; `no_new_privs` must be set.
    pub fn install(profile: Profile) -> Result<(), String> {
        let filter = program(profile);
        let program = libc::sock_fprog {
            len: filter.len() as u16,
            filter: filter.as_ptr() as *mut sock_filter,
        };
        // TSYNC also applies it to threads that already exist.
        let result = unsafe {
            libc::syscall(
                libc::SYS_seccomp,
                libc::SECCOMP_SET_MODE_FILTER,
                libc::SECCOMP_FILTER_FLAG_TSYNC,
                &program,
            )
        };
        match result {
            0 => Ok(()),
            -1 => Err(format!(
                "Cannot install the system call filter: {}",
                std::io::Error::last_os_error()
            )),
            thread => Err(format!(
                "Cannot apply the system call filter to thread {thread}"
            )),
        }
    }
}

#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
pub use imp::install;

/// Applies `Profile::Container` to this process and everything it starts. This
/// also sets `no_new_privs` for them, so set-user-ID programs no longer gain
/// privileges.
pub fn restrict_container() -> Result<(), String> {
    if unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) } != 0 {
        return Err(format!(
            "Cannot set no_new_privs: {}",
            std::io::Error::last_os_error()
        ));
    }
    install(Profile::Container)
}

#[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
pub fn install(_: Profile) -> Result<(), String> {
    Err("System call filters support x86_64 and aarch64 hosts".into())
}

#[cfg(all(test, any(target_arch = "x86_64", target_arch = "aarch64")))]
mod tests {
    use super::*;

    const CHILD: &str = "IVNC_SECCOMP_TEST_CHILD";

    fn errno(result: libc::c_long) -> Option<i32> {
        (result == -1)
            .then(|| std::io::Error::last_os_error().raw_os_error())
            .flatten()
    }

    /// Runs in a separate test process: a filter cannot be removed again.
    fn check_filtered_process(profile: Profile) {
        assert_eq!(
            unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) },
            0
        );
        install(profile).unwrap();
        let application = profile == Profile::Application;
        let missing = c"/nonexistent-ivnc-seccomp-test";
        unsafe {
            // Both profiles deny kernel interfaces outright.
            assert_eq!(
                errno(libc::syscall(
                    libc::SYS_io_uring_setup,
                    1,
                    std::ptr::null_mut::<libc::c_void>(),
                )),
                Some(libc::EPERM)
            );
            assert_eq!(
                errno(libc::syscall(libc::SYS_keyctl, 0, 0, 0, 0, 0)),
                Some(libc::EPERM)
            );
            #[cfg(target_arch = "x86_64")]
            assert_eq!(
                errno(libc::syscall(0x4000_0000 | libc::SYS_getpid)),
                Some(libc::EPERM)
            );
            // Namespaces and mounts only in strict runtimes; the kernel itself
            // rejects a bad descriptor with EBADF.
            assert_eq!(
                errno(libc::syscall(libc::SYS_setns, -1, 0)),
                Some(if application {
                    libc::EPERM
                } else {
                    libc::EBADF
                })
            );
            assert_eq!(
                errno(libc::syscall(
                    libc::SYS_clone3,
                    std::ptr::null::<libc::c_void>(),
                    0
                )) == Some(libc::ENOSYS),
                application
            );
            if application {
                assert_eq!(
                    errno(libc::syscall(
                        libc::SYS_mount,
                        std::ptr::null::<libc::c_char>(),
                        missing.as_ptr(),
                        std::ptr::null::<libc::c_char>(),
                        0,
                        std::ptr::null::<libc::c_void>(),
                    )),
                    Some(libc::EPERM)
                );
                assert_eq!(
                    errno(libc::syscall(libc::SYS_unshare, libc::CLONE_NEWUSER)),
                    Some(libc::EPERM)
                );
                let flags = (libc::CLONE_NEWUSER | libc::SIGCHLD) as libc::c_ulong;
                let child = libc::syscall(libc::SYS_clone, flags, 0, 0, 0, 0);
                if child == 0 {
                    libc::_exit(0);
                }
                assert_eq!(errno(child), Some(libc::EPERM));
            }
        }
        // Threads and processes still start, through the clone fallback.
        assert_eq!(std::thread::spawn(|| 7).join().unwrap(), 7);
        assert!(std::process::Command::new("true")
            .status()
            .unwrap()
            .success());
        assert!(std::fs::read("/proc/self/status").is_ok());
    }

    /// Runs `test` again in a child process that installs `profile` first.
    fn in_filtered_process(test: &str, profile: Profile) {
        if let Some(value) = std::env::var_os(CHILD) {
            assert_eq!(value, format!("{profile:?}").as_str());
            check_filtered_process(profile);
            return;
        }
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                &format!("remote::seccomp::tests::{test}"),
                "--test-threads=1",
            ])
            .env(CHILD, format!("{profile:?}"))
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success(),
            "{stdout}{}",
            String::from_utf8_lossy(&output.stderr)
        );
        // The filtered process must have run the check, not filtered it out.
        assert!(stdout.contains("1 passed"), "{stdout}");
    }

    #[test]
    fn application_filter_blocks_namespaces_and_kernel_interfaces() {
        in_filtered_process(
            "application_filter_blocks_namespaces_and_kernel_interfaces",
            Profile::Application,
        );
    }

    #[test]
    fn container_filter_keeps_namespaces_for_bubblewrap() {
        in_filtered_process(
            "container_filter_keeps_namespaces_for_bubblewrap",
            Profile::Container,
        );
    }

    #[test]
    fn jumps_stay_within_the_program() {
        for profile in [Profile::Container, Profile::Application] {
            let filter = imp::program(profile);
            assert!(filter.len() < 4096);
            for (index, op) in filter.iter().enumerate() {
                if (op.code as u32 & 0x07) == libc::BPF_JMP {
                    assert!(index + 1 + (op.jt.max(op.jf) as usize) < filter.len());
                }
            }
            assert_eq!(filter.last().unwrap().k, libc::SECCOMP_RET_ALLOW);
        }
    }
}
