//! A call's transcript as text: plain, Markdown, subtitles (SubRip, WebVTT) or JSON. Times are
//! from the start of the call.

use recallcraft_model::{Call, Line, Speaker, TranscriptFormat, time};
use serde_json::json;

/// The people who spoke, in the order they first did.
pub fn people(call: &Call, lines: &[Line], your_name: &str) -> Vec<String> {
    let mut seen: Vec<Speaker> = Vec::new();
    for l in lines {
        if !seen.contains(&l.speaker) {
            seen.push(l.speaker);
        }
    }
    seen.into_iter().map(|s| call.speaker_name(s, your_name)).collect()
}

fn offset(call: &Call, ms: u64) -> u64 {
    ms.saturating_sub(call.started_ms)
}

/// The header lines: title, when, how long, who.
fn heading(call: &Call, lines: &[Line], your_name: &str) -> (String, String, String) {
    let when = format!("{} {}", time::local_date(call.started_ms), time::local_time(call.started_ms));
    let long = call.ended_ms.map(|e| time::duration_words(e.saturating_sub(call.started_ms))).unwrap_or_else(|| "still going".to_string());
    let app = if call.app.is_empty() { String::new() } else { format!("{} · ", call.app) };
    (call.display_title(), format!("{app}{when} · {long}"), people(call, lines, your_name).join(", "))
}

/// Text of one line with no line breaks in it (subtitles break on them).
fn one_line(t: &str) -> String {
    t.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn render(format: TranscriptFormat, call: &Call, lines: &[Line], your_name: &str) -> String {
    let name = |l: &Line| call.speaker_name(l.speaker, your_name);
    match format {
        TranscriptFormat::Text => {
            let (title, when, who) = heading(call, lines, your_name);
            let mut s = format!("{title}\n{when}\n");
            if !who.is_empty() {
                s.push_str(&format!("{who}\n"));
            }
            s.push('\n');
            for l in lines {
                s.push_str(&format!("[{}] {}: {}\n", time::clock(offset(call, l.start_ms)), name(l), one_line(&l.text)));
            }
            s
        }
        TranscriptFormat::Markdown => {
            let (title, when, who) = heading(call, lines, your_name);
            let mut s = format!("# {}\n\n*{when}*\n\n", title.replace('#', ""));
            if !who.is_empty() {
                s.push_str(&format!("**People:** {who}\n\n"));
            }
            let mut last: Option<Speaker> = None;
            for l in lines {
                if last == Some(l.speaker) {
                    s.push_str(&format!("{}\n\n", one_line(&l.text)));
                } else {
                    s.push_str(&format!("**{}** · {}\n{}\n\n", name(l), time::clock(offset(call, l.start_ms)), one_line(&l.text)));
                }
                last = Some(l.speaker);
            }
            s
        }
        TranscriptFormat::Srt => {
            let mut s = String::new();
            for (i, l) in lines.iter().enumerate() {
                s.push_str(&format!(
                    "{}\n{} --> {}\n{}: {}\n\n",
                    i + 1,
                    time::subtitle_time(offset(call, l.start_ms), ','),
                    time::subtitle_time(offset(call, l.end_ms), ','),
                    name(l),
                    one_line(&l.text)
                ));
            }
            s
        }
        TranscriptFormat::Vtt => {
            let mut s = String::from("WEBVTT\n\n");
            for l in lines {
                let who = name(l).replace(['<', '>'], "");
                s.push_str(&format!(
                    "{} --> {}\n<v {who}>{}\n\n",
                    time::subtitle_time(offset(call, l.start_ms), '.'),
                    time::subtitle_time(offset(call, l.end_ms), '.'),
                    one_line(&l.text).replace('<', "&lt;").replace('>', "&gt;")
                ));
            }
            s
        }
        TranscriptFormat::Json => {
            let items: Vec<serde_json::Value> = lines
                .iter()
                .map(|l| {
                    json!({
                        "start_ms": l.start_ms,
                        "end_ms": l.end_ms,
                        "offset_ms": offset(call, l.start_ms),
                        "speaker": l.speaker.key(),
                        "name": name(l),
                        "text": l.text,
                    })
                })
                .collect();
            let v = json!({
                "id": call.id,
                "title": call.display_title(),
                "app": call.app,
                "started_ms": call.started_ms,
                "ended_ms": call.ended_ms,
                "people": people(call, lines, your_name),
                "lines": items,
            });
            serde_json::to_string_pretty(&v).unwrap_or_default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> (Call, Vec<Line>) {
        let mut call = Call {
            id: "c".into(),
            app: "Microsoft Teams".into(),
            title: "Weekly sync".into(),
            started_ms: 1_000_000,
            ended_ms: Some(1_000_000 + 125_000),
            ..Call::default()
        };
        call.set_name(Speaker::Voice(1), "John");
        let lines = vec![
            Line { start_ms: 1_002_000, end_ms: 1_004_500, speaker: Speaker::Voice(1), text: "Are you the developer\non this?".into() },
            Line { start_ms: 1_005_000, end_ms: 1_007_000, speaker: Speaker::Me, text: "Yes, I am.".into() },
            Line { start_ms: 1_008_000, end_ms: 1_009_000, speaker: Speaker::Me, text: "<b>Why?</b>".into() },
        ];
        (call, lines)
    }

    #[test]
    fn every_format_names_who_spoke() {
        let (call, lines) = sample();
        let text = render(TranscriptFormat::Text, &call, &lines, "Bill");
        assert!(text.starts_with("Weekly sync\nMicrosoft Teams · "));
        assert!(text.contains("John, Bill\n"));
        assert!(text.contains("[0:02] John: Are you the developer on this?\n"));
        let md = render(TranscriptFormat::Markdown, &call, &lines, "Bill");
        assert!(md.starts_with("# Weekly sync\n") && md.contains("**Bill** · 0:05\nYes, I am.\n\n<b>Why?</b>"));
        let srt = render(TranscriptFormat::Srt, &call, &lines, "Bill");
        assert!(srt.starts_with("1\n00:00:02,000 --> 00:00:04,500\nJohn: Are you the developer on this?\n\n2\n"));
        let vtt = render(TranscriptFormat::Vtt, &call, &lines, "Bill");
        assert!(vtt.starts_with("WEBVTT\n\n00:00:02.000 --> 00:00:04.500\n<v John>Are you"));
        assert!(vtt.contains("&lt;b&gt;Why?"));
        let json: serde_json::Value = serde_json::from_str(&render(TranscriptFormat::Json, &call, &lines, "Bill")).unwrap();
        assert_eq!(json["lines"][0]["name"], "John");
        assert_eq!(json["lines"][1]["offset_ms"], 5000);
        assert_eq!(render(TranscriptFormat::Srt, &call, &[], ""), "");
    }
}
