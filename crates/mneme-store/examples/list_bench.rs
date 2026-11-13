//! Times the calls that scan the meetings root, on generated meetings in a
//! temporary folder.
//!
//! ```text
//! cargo run -p mneme-store --release --example list_bench            # 500 meetings
//! cargo run -p mneme-store --release --example list_bench -- 2000
//! ```
//!
//! The folders are written straight to disk in the shape `create_meeting`
//! gives them (the `notes.md` of one real meeting is the template), each with
//! about 1.5 KB of notes and a 200-line transcript. The files were written a
//! moment earlier, so they are in the operating system's cache: this measures
//! a warm scan, not the first one after a reboot.
//!
//! Nothing here is asserted beyond the counts; read the numbers together with
//! the machine's load at the time (`uptime`).

use std::fs;
use std::path::Path;
use std::time::{Duration, Instant};

use mneme_store::{MeetingId, NewMeeting, StartTime, Store};

fn main() {
    let count: usize = std::env::args()
        .nth(1)
        .map(|arg| arg.parse().expect("the argument is a number of meetings"))
        .unwrap_or(500);

    let dir = tempfile::tempdir().expect("a temp folder");
    let root = dir.path().join("Mneme");
    let ids = generate(&root, count);
    println!("{count} meetings in {}", root.display());
    println!();
    println!("{:<46} {:>9} {:>9} {:>9}", "call", "min", "median", "max");

    // The first list of a store that has never seen the root: what a launch
    // does.
    let mut first_lists = Vec::new();
    for _ in 0..20 {
        let store = Store::open(&root).unwrap();
        let started = Instant::now();
        let list = store.list_meetings().unwrap();
        first_lists.push(started.elapsed());
        assert_eq!(list.meetings.len(), count);
        assert!(list.warnings.is_empty(), "{:?}", list.warnings.first());
    }
    report("list_meetings, new store", &mut first_lists);

    let store = Store::open(&root).unwrap();
    report(
        "list_meetings, same store",
        &mut time(20, || {
            assert_eq!(store.list_meetings().unwrap().meetings.len(), count);
        }),
    );

    // Recovery when nothing was left open: the scan plus nothing.
    report(
        "recover, nothing to do",
        &mut time(20, || {
            assert!(store.recover().unwrap().nothing_to_do());
        }),
    );

    // Launch as issue 021 will do it: recovery, then the first list.
    let mut launches = Vec::new();
    for _ in 0..20 {
        let store = Store::open(&root).unwrap();
        let started = Instant::now();
        store.recover().unwrap();
        store.list_meetings().unwrap();
        launches.push(started.elapsed());
    }
    report("recover + list_meetings, new store", &mut launches);

    // Find by id. A new store has to scan; one that has scanned goes
    // straight to the folder and checks it.
    let id = &ids[count / 2];
    let mut scans = Vec::new();
    for _ in 0..20 {
        let store = Store::open(&root).unwrap();
        let started = Instant::now();
        store.meeting(id).unwrap();
        scans.push(started.elapsed());
    }
    report("meeting(id), new store (scan)", &mut scans);
    report(
        "meeting(id), same store (hint)",
        &mut time(200, || {
            store.meeting(id).unwrap();
        }),
    );
    report(
        "load_meeting(id), same store",
        &mut time(200, || {
            store.load_meeting(id).unwrap();
        }),
    );

    // A notes save, which syncs to disk, with and without the scan.
    let mut n = 0;
    report(
        "save_notes, same store (hint)",
        &mut time(20, || {
            n += 1;
            store.save_notes(id, &format!("- save {n}\n")).unwrap();
        }),
    );
    let mut saves = Vec::new();
    for n in 0..20 {
        let store = Store::open(&root).unwrap();
        let started = Instant::now();
        store.save_notes(id, &format!("- cold save {n}\n")).unwrap();
        saves.push(started.elapsed());
    }
    report("save_notes, new store (scan)", &mut saves);
}

/// Runs `call` `times` times and returns how long each run took.
fn time(times: usize, mut call: impl FnMut()) -> Vec<Duration> {
    (0..times)
        .map(|_| {
            let started = Instant::now();
            call();
            started.elapsed()
        })
        .collect()
}

fn report(label: &str, samples: &mut [Duration]) {
    samples.sort();
    let ms = |d: Duration| format!("{:.2} ms", d.as_secs_f64() * 1000.0);
    println!(
        "{label:<46} {:>9} {:>9} {:>9}",
        ms(samples[0]),
        ms(samples[samples.len() / 2]),
        ms(samples[samples.len() - 1])
    );
}

/// Writes `count` finished meetings under `root` and returns their ids,
/// newest first.
fn generate(root: &Path, count: usize) -> Vec<MeetingId> {
    // One real meeting, made and finished by the store, is the template for
    // the rest, so the generated files have the shape the store writes.
    let store = Store::open(root).unwrap();
    let newest: StartTime = "2026-10-27T14:30:00+01:00".parse().unwrap();
    let template = store
        .create_meeting(NewMeeting {
            started_at: newest,
            model: Some("large-v3-turbo".to_owned()),
            language: Some("auto".to_owned()),
        })
        .unwrap();
    let template = store.finish_meeting(&template.id, 1800).unwrap().meeting;
    let template_notes = fs::read_to_string(template.notes_path()).unwrap();
    let template_date = template.started_at.to_rfc3339();
    assert!(template_notes.contains(template.id.as_str()));
    assert!(template_notes.contains(&template_date));

    let body: String = (0..30)
        .map(|line| format!("- a note line of ordinary length, number {line} of thirty\n"))
        .collect();
    let transcript: String = (0..200)
        .map(|line| {
            format!(
                "[00:{:02}:{:02}] {}: A sentence of about the length people say in one breath.\n",
                line * 9 / 60,
                line * 9 % 60,
                if line % 2 == 0 { "Them" } else { "Me" }
            )
        })
        .collect();

    let mut ids = vec![template.id.clone()];
    fs::write(template.notes_path(), format!("{template_notes}{body}")).unwrap();
    fs::write(template.transcript_path(), &transcript).unwrap();
    for n in 1..count {
        // One meeting every 3 hours 7 minutes going back: several a day,
        // each in a minute of its own.
        let minutes_back = n as i64 * 187;
        let (day, minute_of_day) = (
            minutes_back.div_euclid(1440),
            1439 - minutes_back.rem_euclid(1440),
        );
        // Dates are day numbers counted back from 2026-10-26, written out
        // through a month table so the example needs no date library.
        let (year, month, day) = days_before_2026_10_27(day + 1);
        let (hour, minute) = (minute_of_day / 60, minute_of_day % 60);
        let date = format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:00+01:00");
        let id = MeetingId::new(format!("00000000-0000-4000-8000-{n:012}"));
        let folder = root.join(format!(
            "{year:04}-{month:02}-{day:02} {hour:02}{minute:02} Meeting"
        ));
        fs::create_dir(&folder).unwrap();
        let notes = template_notes
            .replace(template.id.as_str(), id.as_str())
            .replace(&template_date, &date);
        fs::write(folder.join("notes.md"), format!("{notes}{body}")).unwrap();
        fs::write(folder.join("transcript.md"), &transcript).unwrap();
        fs::write(folder.join("notes-timeline.jsonl"), "").unwrap();
        ids.push(id);
    }
    ids
}

/// The date `days` days before 2026-10-27.
fn days_before_2026_10_27(days: i64) -> (i64, i64, i64) {
    let is_leap = |year: i64| year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days_in = |year: i64, month: i64| match month {
        2 if is_leap(year) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    };
    let (mut year, mut month, mut day) = (2026, 10, 27);
    for _ in 0..days {
        day -= 1;
        if day == 0 {
            month -= 1;
            if month == 0 {
                month = 12;
                year -= 1;
            }
            day = days_in(year, month);
        }
    }
    (year, month, day)
}
