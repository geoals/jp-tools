//! Where one sitting ends and the next begins.

use serde::Serialize;

use super::line::LineEvent;
use super::presence::Presence;

#[derive(Debug, Serialize, PartialEq)]
pub struct Session {
    pub start_ts: f64,
    pub end_ts: f64,
    pub chars: i64,
    pub active_secs: f64,
    pub lines: i64,
}

/// Split a time-ordered line stream into sessions and derive active reading
/// time. Each inter-line gap credits reading time per `presence`; a gap above
/// `session_gap_secs` closes the session. A lone line yields a session with
/// zero active time — credit comes from gaps, not line count.
pub fn derive_sessions(
    lines: &[LineEvent],
    presence: &Presence,
    session_gap_secs: f64,
) -> Vec<Session> {
    let mut out: Vec<Session> = Vec::new();
    for (k, line) in lines.iter().enumerate() {
        match out.last_mut() {
            Some(s) if line.ts - s.end_ts <= session_gap_secs => {
                let prev = &lines[k - 1];
                s.active_secs += presence.credit(prev, line.ts - prev.ts);
                s.end_ts = line.ts;
                s.chars += line.chars;
                s.lines += 1;
            }
            _ => out.push(Session {
                start_ts: line.ts,
                end_ts: line.ts,
                chars: line.chars,
                active_secs: 0.0,
                lines: 1,
            }),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::super::presence::measure_pace;
    use super::super::testutil::ev;
    use super::*;

    #[test]
    fn sessions_split_on_gap_and_cap_afk() {
        let lines = [ev(0.0, 10), ev(30.0, 20), ev(330.0, 5), ev(1000.0, 7)];
        // afk cap 120s, session gap 600s: the 300s gap stays in-session but is
        // not credited whole; the 670s gap starts a new session.
        let sessions = derive_sessions(
            &lines,
            &Presence::new(&[], measure_pace(&lines, &[], 120.0), 120.0),
            600.0,
        );
        assert_eq!(sessions.len(), 2);
        assert_eq!(sessions[0].chars, 35);
        assert_eq!(sessions[0].lines, 3);
        // Pace comes from the one undisputed gap: 10 chars in 30s. The second
        // line's 20 chars are therefore worth 60s, and that is what its 300s
        // gap earns — the remaining four minutes are absence. The old flat cap
        // would have credited 120s here on no evidence at all.
        assert_eq!(sessions[0].active_secs, 30.0 + 60.0);
        assert_eq!(sessions[0].end_ts, 330.0);
        assert_eq!(sessions[1].chars, 7);
        assert_eq!(sessions[1].active_secs, 0.0);
    }
}
