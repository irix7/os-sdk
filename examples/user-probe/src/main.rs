// SPDX-License-Identifier: GPL-3.0-only
//! Example static n32 user image: file system, directory and clock probes.
//!
//! The namespace is the kernel's bring-up fixture, not a general filesystem.
//! The probe reports what it finds and returns a non-zero status when an
//! expected fixture is missing.  It is a host build/link target in CI; it has
//! not been run on an Indy.

#![no_std]
#![no_main]

use user_sdk::abi::{DT_CHR, DT_DIR, DT_REG};
use user_sdk::fs::{self, Dir, File};
use user_sdk::time;
use user_sdk::{entry, panic_handler, println, Context, O_RDONLY};

fn run(context: &Context) -> i32 {
    let mut failures = 0;

    match fs::stat(b"/hello.txt") {
        Some(stat) => {
            println!(
                "probe: stat /hello.txt size={} mode={:#o}",
                stat.size(),
                stat.st_mode
            );
        }
        None => {
            println!("probe: stat /hello.txt failed");
            failures += 1;
        }
    }

    match File::open(b"/etc/motd", O_RDONLY) {
        Some(mut file) => {
            let mut buffer = [0u8; 512];
            let count = file.read(&mut buffer);
            if count > 0 {
                context.write(b"probe: /etc/motd: ");
                context.write(&buffer[..count as usize]);
            } else {
                println!("probe: read /etc/motd failed");
                failures += 1;
            }
        }
        None => {
            println!("probe: open /etc/motd failed");
            failures += 1;
        }
    }

    let mut cwd = [0u8; 64];
    match fs::getcwd(&mut cwd) {
        Some(path) => println!(
            "probe: cwd={}",
            core::str::from_utf8(path).unwrap_or("<non-utf8>")
        ),
        None => {
            println!("probe: getcwd failed");
            failures += 1;
        }
    }

    match Dir::open(b"/") {
        Some(mut directory) => {
            let mut name = [0u8; 64];
            let mut entries = 0;
            while let Some((kind, entry)) = directory.next(&mut name) {
                context.write(b"probe: dir ");
                context.write(entry);
                context.write(b" type=");
                let type_name = match kind {
                    DT_DIR => b"dir".as_slice(),
                    DT_REG => b"file".as_slice(),
                    DT_CHR => b"char".as_slice(),
                    _ => b"other".as_slice(),
                };
                context.write(type_name);
                context.write(b"\n");
                entries += 1;
            }
            println!("probe: root entries={}", entries);
        }
        None => {
            println!("probe: open / failed");
            failures += 1;
        }
    }

    match time::realtime() {
        Some(value) => println!(
            "probe: realtime sec={} nsec={}",
            value.tv_sec, value.tv_nsec
        ),
        None => {
            println!("probe: realtime failed");
            failures += 1;
        }
    }
    match time::monotonic() {
        Some(value) => println!(
            "probe: monotonic sec={} nsec={}",
            value.tv_sec, value.tv_nsec
        ),
        None => {
            println!("probe: monotonic failed");
            failures += 1;
        }
    }
    if time::sleep(0, 1_000_000) {
        println!("probe: nanosleep 1ms ok");
    } else {
        println!("probe: nanosleep failed");
        failures += 1;
    }

    println!("probe: done failures={}", failures);
    failures
}

entry!(run);
panic_handler!();
