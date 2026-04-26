use once_cell::sync::Lazy;
use regex::Regex;

static ON_DATE_WROTE: Lazy<Regex> = Lazy::new(|| {
    // "On <date>, <name> <email|>(?) wrote:" — match start of any line.
    Regex::new(r"(?m)^On .{1,200}\bwrote:\s*$").unwrap()
});

static OUTLOOK_SEP: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?m)^-{3,}\s*Original Message\s*-{3,}\s*$").unwrap());

/// Returns only the new content of an email body — drops everything from the
/// first quoted-reply marker onward, plus any line that begins with '>'.
pub fn strip_quoted_reply(body: &str) -> String {
    // Cut at the earliest of the two marker types.
    let mut cut: Option<usize> = None;
    for re in [&*ON_DATE_WROTE, &*OUTLOOK_SEP] {
        if let Some(m) = re.find(body) {
            cut = Some(cut.map_or(m.start(), |c| c.min(m.start())));
        }
    }
    let head = if let Some(idx) = cut {
        &body[..idx]
    } else {
        body
    };

    // Drop any line starting with '>'.
    let kept: Vec<&str> = head
        .lines()
        .filter(|l| !l.trim_start().starts_with('>'))
        .collect();
    kept.join("\n").trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_on_date_wrote_block_and_below() {
        let input = "Quick reply.\n\nOn Mon, Apr 21, 2026 at 9:14 AM, Sarah <sarah@x> wrote:\n> earlier text\n>> deeper\n";
        assert_eq!(strip_quoted_reply(input), "Quick reply.");
    }

    #[test]
    fn strips_lines_starting_with_gt() {
        let input = "new thought\n> old thought\n> > older\nnewer\n";
        assert_eq!(strip_quoted_reply(input), "new thought\nnewer");
    }

    #[test]
    fn strips_outlook_original_message_separator() {
        let input =
            "reply text\n\n-----Original Message-----\nFrom: a@b\nTo: c@d\nSubject: ...\nbody";
        assert_eq!(strip_quoted_reply(input), "reply text");
    }

    #[test]
    fn passthrough_when_no_quote_found() {
        let s = "single paragraph no markers";
        assert_eq!(strip_quoted_reply(s), s);
    }
}
