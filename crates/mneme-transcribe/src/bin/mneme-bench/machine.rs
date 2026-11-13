//! What the benchmark ran on: chip, cores, memory, macOS version, and how
//! busy the machine was. Read with `sysctl` and `sw_vers`; anything that
//! cannot be read is left out of the report.

use std::process::Command;

use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Machine {
    pub chip: Option<String>,
    pub cores: Option<u32>,
    pub performance_cores: Option<u32>,
    pub efficiency_cores: Option<u32>,
    pub memory_bytes: Option<u64>,
    pub os_version: Option<String>,
}

impl Machine {
    pub fn read() -> Self {
        Machine {
            chip: sysctl("machdep.cpu.brand_string"),
            cores: sysctl("hw.physicalcpu").and_then(|text| text.parse().ok()),
            performance_cores: sysctl("hw.perflevel0.physicalcpu")
                .and_then(|text| text.parse().ok()),
            efficiency_cores: sysctl("hw.perflevel1.physicalcpu")
                .and_then(|text| text.parse().ok()),
            memory_bytes: sysctl("hw.memsize").and_then(|text| text.parse().ok()),
            os_version: output("/usr/bin/sw_vers", &["-productVersion"])
                .map(|version| format!("macOS {version}")),
        }
    }
}

/// The local date and time, such as `2026-11-12 21:03:11 +0100`.
pub fn local_time() -> Option<String> {
    output("/bin/date", &["+%Y-%m-%d %H:%M:%S %z"])
}

/// The system load averages over the last 1, 5 and 15 minutes: the number
/// of threads running or waiting for a core. Far above the number of cores
/// means other work was competing with the benchmark.
pub fn load_average() -> Option<[f64; 3]> {
    parse_load_average(&sysctl("vm.loadavg")?)
}

/// Parses `sysctl -n vm.loadavg`, which prints `{ 1.95 2.10 2.31 }`.
fn parse_load_average(text: &str) -> Option<[f64; 3]> {
    let mut numbers = text
        .trim()
        .trim_start_matches('{')
        .trim_end_matches('}')
        .split_whitespace()
        .map(|number| number.replace(',', ".").parse::<f64>());
    let averages = [
        numbers.next()?.ok()?,
        numbers.next()?.ok()?,
        numbers.next()?.ok()?,
    ];
    numbers.next().is_none().then_some(averages)
}

fn sysctl(name: &str) -> Option<String> {
    output("/usr/sbin/sysctl", &["-n", name])
}

fn output(program: &str, arguments: &[&str]) -> Option<String> {
    let output = Command::new(program).args(arguments).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?.trim().to_owned();
    (!text.is_empty()).then_some(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_averages_parse_from_sysctl_s_format() {
        assert_eq!(
            parse_load_average("{ 1.95 2.10 12.31 }\n"),
            Some([1.95, 2.10, 12.31])
        );
        // Some locales print a decimal comma.
        assert_eq!(
            parse_load_average("{ 1,95 2,10 2,31 }"),
            Some([1.95, 2.10, 2.31])
        );
        assert_eq!(parse_load_average(""), None);
        assert_eq!(parse_load_average("{ 1.0 2.0 }"), None);
        assert_eq!(parse_load_average("{ 1.0 2.0 3.0 4.0 }"), None);
        assert_eq!(parse_load_average("{ a b c }"), None);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn this_mac_can_be_described() {
        let machine = Machine::read();
        assert!(machine.chip.is_some_and(|chip| !chip.is_empty()));
        assert!(machine.cores.is_some_and(|cores| cores >= 1));
        assert!(machine.memory_bytes.is_some_and(|bytes| bytes > 1 << 30));
        assert!(
            machine
                .os_version
                .is_some_and(|os| os.starts_with("macOS "))
        );
        assert!(load_average().is_some());
        // 2026-11-12 21:03:11 +0100
        let time = local_time().unwrap();
        assert_eq!(time.len(), 25, "{time}");
        assert_eq!(&time[4..5], "-");
        assert_eq!(&time[13..14], ":");
    }
}
