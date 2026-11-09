#![cfg(unix)]
//! TTY-only lifecycle fixture. The child test enters a real `TerminalSession`
//! on a PTY and then panics; the parent drains the PTY concurrently and
//! verifies the PTY is back in cooked mode. Run explicitly:
//! `cargo test --bin rous frontend::tui::tests::pty::pty_panic_restores_terminal -- --ignored --exact`

use std::os::fd::FromRawFd;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[test]
#[ignore = "TTY-only lifecycle fixture: run with --ignored --exact frontend::tui::tests::pty::pty_panic_restores_terminal"]
fn pty_panic_restores_terminal() {
    unsafe {
        let (mut master, mut slave): (libc::c_int, libc::c_int) = (-1, -1);
        assert_eq!(
            libc::openpty(
                &mut master,
                &mut slave,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            ),
            0,
            "openpty"
        );
        for fd in [master, slave] {
            libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC);
        }
        let mut size: libc::winsize = std::mem::zeroed();
        size.ws_row = 24;
        size.ws_col = 80;
        libc::ioctl(slave, libc::TIOCSWINSZ, &size);

        // Drain the master while the child runs; otherwise a panic backtrace
        // can fill the PTY buffer and block the child before it exits.
        let reader = std::thread::spawn(move || {
            let mut master = std::fs::File::from_raw_fd(master);
            let mut output = Vec::new();
            let _ = std::io::Read::read_to_end(&mut master, &mut output);
            output
        });

        let child_in = libc::dup(slave);
        let child_out = libc::dup(slave);
        let child_err = libc::dup(slave);
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--ignored",
                "--exact",
                "--test-threads=1",
                "--nocapture",
                "frontend::tui::tests::pty::pty_panic_child",
            ])
            .env_remove("SAKI_THEME")
            .stdin(Stdio::from_raw_fd(child_in))
            .stdout(Stdio::from_raw_fd(child_out))
            .stderr(Stdio::from_raw_fd(child_err))
            .spawn()
            .expect("spawn PTY child");

        // Bounded wait so a wedged child cannot hang the fixture forever.
        let deadline = Instant::now() + Duration::from_secs(30);
        let status = loop {
            match child.try_wait().expect("try_wait") {
                Some(status) => break status,
                None if Instant::now() >= deadline => {
                    let _ = child.kill();
                    let _ = child.wait();
                    libc::close(slave);
                    let _ = reader.join();
                    panic!("PTY child did not exit within the bounded wait");
                }
                None => std::thread::sleep(Duration::from_millis(25)),
            }
        };
        assert!(!status.success(), "the child test panics by design");

        let mut termios: libc::termios = std::mem::zeroed();
        assert_eq!(libc::tcgetattr(slave, &mut termios), 0, "tcgetattr");
        assert_ne!(
            termios.c_lflag & libc::ICANON,
            0,
            "ICANON must be restored after panic"
        );
        assert_ne!(
            termios.c_lflag & libc::ECHO,
            0,
            "ECHO must be restored after panic"
        );
        // Closing the last slave descriptor lets the drain thread reach EOF.
        libc::close(slave);
        let output = reader.join().expect("PTY drain thread");
        let text = String::from_utf8_lossy(&output);
        assert!(
            text.contains("pty lifecycle panic"),
            "child panic output captured: {text}"
        );
    }
}

#[test]
#[ignore = "TTY-only lifecycle child invoked by pty_panic_restores_terminal"]
fn pty_panic_child() {
    let _session = crate::frontend::tui::terminal::TerminalSession::enter(
        crate::frontend::tui::theme::ThemeSetting::Light,
    )
    .expect("enter terminal session on the PTY");
    panic!("pty lifecycle panic");
}

/// Parent fixture for deterministic reader cancellation and suspend/resume.
/// The child drops a pending `next_input()` future, prints a marker, waits for
/// a real key, then verifies cooked termios during suspension and raw after
/// resume. Coordination is by observed output, not sleeps.
#[test]
#[ignore = "TTY-only fixture: run with --ignored --exact frontend::tui::tests::pty::pty_reader_cancel_and_suspend"]
fn pty_reader_cancel_and_suspend() {
    use std::io::{Read, Write};
    use std::os::fd::FromRawFd;
    use std::sync::mpsc;
    use std::time::Duration;

    unsafe {
        let (mut master, mut slave): (libc::c_int, libc::c_int) = (-1, -1);
        assert_eq!(
            libc::openpty(
                &mut master,
                &mut slave,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            ),
            0,
            "openpty"
        );
        for fd in [master, slave] {
            libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC);
        }
        let mut size: libc::winsize = std::mem::zeroed();
        size.ws_row = 24;
        size.ws_col = 80;
        libc::ioctl(slave, libc::TIOCSWINSZ, &size);

        let (chunks_tx, chunks_rx) = mpsc::channel::<Vec<u8>>();
        let reader = std::thread::spawn(move || {
            let mut master = std::fs::File::from_raw_fd(master);
            let mut buffer = [0u8; 4096];
            loop {
                match master.read(&mut buffer) {
                    Ok(0) | Err(_) => break,
                    Ok(read) => {
                        if chunks_tx.send(buffer[..read].to_vec()).is_err() {
                            break;
                        }
                    }
                }
            }
        });

        let child_in = libc::dup(slave);
        let child_out = libc::dup(slave);
        let child_err = libc::dup(slave);
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--ignored",
                "--exact",
                "--test-threads=1",
                "--nocapture",
                "frontend::tui::tests::pty::pty_reader_cancel_child",
            ])
            .env_remove("SAKI_THEME")
            .stdin(Stdio::from_raw_fd(child_in))
            .stdout(Stdio::from_raw_fd(child_out))
            .stderr(Stdio::from_raw_fd(child_err))
            .spawn()
            .expect("spawn PTY reader child");

        let mut output = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut cancel_key_written = false;
        let mut resume_key_written = false;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            assert!(
                !remaining.is_zero(),
                "PTY reader child timed out: {}",
                String::from_utf8_lossy(&output)
            );
            if let Ok(chunk) = chunks_rx.recv_timeout(remaining.min(Duration::from_millis(200))) {
                output.extend_from_slice(&chunk);
            }
            let text = String::from_utf8_lossy(&output);
            if !cancel_key_written && text.contains("CANCEL_READY") {
                let mut master = std::fs::File::from_raw_fd(libc::dup(master));
                master.write_all(b"k").expect("write key to PTY");
                cancel_key_written = true;
            }
            if !resume_key_written && text.contains("RESUMED") {
                let mut master = std::fs::File::from_raw_fd(libc::dup(master));
                master.write_all(b"z").expect("write resume key to PTY");
                resume_key_written = true;
            }
            if text.contains("READER_DONE") {
                break;
            }
            if let Some(status) = child.try_wait().expect("try_wait") {
                assert!(
                    status.success(),
                    "reader child exited early: {status}\n{}",
                    String::from_utf8_lossy(&output)
                );
                break;
            }
        }
        let status = child.wait().expect("wait reader child");
        assert!(status.success(), "reader child failed: {status}");
        let text = String::from_utf8_lossy(&output);
        assert!(text.contains("CANCEL_READY"), "{text}");
        assert!(text.contains("PAUSE_READY"), "{text}");
        assert!(text.contains("RESUMED"), "{text}");
        assert!(text.contains("READER_DONE"), "{text}");

        let mut termios: libc::termios = std::mem::zeroed();
        assert_eq!(libc::tcgetattr(slave, &mut termios), 0, "tcgetattr");
        assert_ne!(termios.c_lflag & libc::ICANON, 0);
        assert_ne!(termios.c_lflag & libc::ECHO, 0);

        libc::close(slave);
        let _ = reader.join();
    }
}

#[test]
#[ignore = "TTY-only child invoked by pty_reader_cancel_and_suspend"]
fn pty_reader_cancel_child() {
    use std::time::Duration;

    let mut session = crate::frontend::tui::terminal::TerminalSession::enter(
        crate::frontend::tui::theme::ThemeSetting::Light,
    )
    .expect("enter terminal session on the PTY");
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap();
    runtime.block_on(async {
        // Poll once so the worker has an outstanding read, then drop the future.
        let mut pending = Box::pin(session.next_input());
        std::future::poll_fn(|context| {
            let _ = std::future::Future::poll(pending.as_mut(), context);
            std::task::Poll::Ready(())
        })
        .await;
        drop(pending);
        println!("CANCEL_READY");
        let event = tokio::time::timeout(Duration::from_secs(10), session.next_input())
            .await
            .expect("cancelled request must still deliver the next event")
            .expect("input io")
            .expect("input stream");
        match event {
            crate::frontend::tui::terminal_input::Input::Event(crossterm::event::Event::Key(
                key,
            )) if key.code == crossterm::event::KeyCode::Char('k') => {}
            _ => panic!("expected the first real key to survive cancellation"),
        }
        // Leave an outstanding blocked poll, then suspend: stop must wake and
        // join it deterministically. No input arrives for this request.
        let mut paused = Box::pin(session.next_input());
        std::future::poll_fn(|context| {
            let _ = std::future::Future::poll(paused.as_mut(), context);
            std::task::Poll::Ready(())
        })
        .await;
        drop(paused);
        println!("PAUSE_READY");
    });

    session.suspend_external().expect("suspend terminal");
    let mut cooked: libc::termios = unsafe { std::mem::zeroed() };
    assert_eq!(unsafe { libc::tcgetattr(0, &mut cooked) }, 0, "tcgetattr");
    assert_ne!(cooked.c_lflag & libc::ICANON, 0, "ICANON during editor");
    assert_ne!(cooked.c_lflag & libc::ECHO, 0, "ECHO during editor");
    session.resume_external().expect("resume terminal");
    let mut raw: libc::termios = unsafe { std::mem::zeroed() };
    assert_eq!(unsafe { libc::tcgetattr(0, &mut raw) }, 0, "tcgetattr");
    assert_eq!(raw.c_lflag & libc::ICANON, 0, "raw after resume");
    println!("RESUMED");
    runtime.block_on(async {
        let event = tokio::time::timeout(Duration::from_secs(10), session.next_input())
            .await
            .expect("post-resume key must arrive")
            .expect("input io")
            .expect("input stream");
        match event {
            crate::frontend::tui::terminal_input::Input::Event(crossterm::event::Event::Key(
                key,
            )) if key.code == crossterm::event::KeyCode::Char('z') => {}
            _ => panic!("expected the first post-resume key"),
        }
    });
    drop(session);
    println!("READER_DONE");
}
