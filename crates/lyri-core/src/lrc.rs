use std::time::Duration;

use crate::lyrics::TimedLine;

pub fn parse_synced_lrc(input: &str) -> Vec<TimedLine> {
    let mut lines = Vec::new();

    for raw_line in input.lines() {
        let mut rest = raw_line.trim();
        let mut stamps = Vec::new();

        while let Some(after_open) = rest.strip_prefix('[') {
            let Some(close) = after_open.find(']') else {
                break;
            };

            let stamp = &after_open[..close];
            if let Some(time) = parse_timestamp(stamp) {
                stamps.push(time);
                rest = after_open[close + 1..].trim_start();
            } else {
                break;
            }
        }

        if stamps.is_empty() {
            continue;
        }

        let text = rest.trim();
        lines.extend(stamps.into_iter().map(|timestamp| TimedLine {
            timestamp,
            text: text.to_string(),
        }));
    }

    lines.sort_by_key(|line| line.timestamp);
    lines
}

fn parse_timestamp(stamp: &str) -> Option<Duration> {
    let (minutes, rest) = stamp.split_once(':')?;
    let minutes = minutes.parse::<u64>().ok()?;

    let (seconds, fraction) = rest.split_once('.').unwrap_or((rest, ""));

    let seconds = seconds.parse::<u64>().ok()?;
    if seconds >= 60 {
        return None;
    }

    let millis = if fraction.is_empty() {
        0
    } else if fraction.chars().all(|c| c.is_ascii_digit()) {
        let digits = fraction.chars().take(3).collect::<String>();
        format!("{digits:0<3}").parse::<u64>().ok()?
    } else {
        return None;
    };

    let total_millis = minutes
        .checked_mul(60)?
        .checked_add(seconds)?
        .checked_mul(1000)?
        .checked_add(millis)?;
    Some(Duration::from_millis(total_millis))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_basic_lrc() {
        let lines = parse_synced_lrc("[00:01.25] hello\n[00:02.500] world");

        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].timestamp, Duration::from_millis(1250));
        assert_eq!(lines[0].text, "hello");
        assert_eq!(lines[1].timestamp, Duration::from_millis(2500));
    }

    #[test]
    fn supports_repeated_timestamps_and_blank_text() {
        let lines = parse_synced_lrc("[00:01][00:02] same\n[00:03.1] ");

        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0].text, "same");
        assert_eq!(lines[1].text, "same");
        assert_eq!(lines[2].timestamp, Duration::from_millis(3100));
        assert_eq!(lines[2].text, "");
    }

    #[test]
    fn ignores_malformed_lines() {
        let lines = parse_synced_lrc("[ar:meta]\nnot timed\n[00:61.00] bad\n[01:00.00] good");

        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].timestamp, Duration::from_secs(60));
        assert_eq!(lines[0].text, "good");
    }
}
