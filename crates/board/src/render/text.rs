//! Text measurement and wrapping for the renderer (spec section 8.1).
//!
//! SVG can't wrap text, so the renderer does it here: glyph advances from the
//! bundled Inter, greedy breaks at spaces, long words broken by character.
//! If text still doesn't fit its box, the size steps down to a minimum, and
//! then the last line is cut with an ellipsis. Good for Latin text; complex
//! scripts are out of scope for v1.

use std::sync::OnceLock;

use ttf_parser::Face;

pub static INTER_REGULAR: &[u8] = include_bytes!("../../fonts/Inter-Regular.ttf");
pub static INTER_SEMIBOLD: &[u8] = include_bytes!("../../fonts/Inter-SemiBold.ttf");
pub static INTER_BOLD: &[u8] = include_bytes!("../../fonts/Inter-Bold.ttf");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Weight {
    Regular,
    SemiBold,
    Bold,
}

impl Weight {
    pub fn css(self) -> u16 {
        match self {
            Self::Regular => 400,
            Self::SemiBold => 600,
            Self::Bold => 700,
        }
    }
}

struct Fonts {
    regular: Face<'static>,
    semibold: Face<'static>,
    bold: Face<'static>,
}

fn fonts() -> &'static Fonts {
    static FONTS: OnceLock<Fonts> = OnceLock::new();
    FONTS.get_or_init(|| Fonts {
        regular: Face::parse(INTER_REGULAR, 0).expect("bundled Inter Regular"),
        semibold: Face::parse(INTER_SEMIBOLD, 0).expect("bundled Inter SemiBold"),
        bold: Face::parse(INTER_BOLD, 0).expect("bundled Inter Bold"),
    })
}

fn face(weight: Weight) -> &'static Face<'static> {
    let f = fonts();
    match weight {
        Weight::Regular => &f.regular,
        Weight::SemiBold => &f.semibold,
        Weight::Bold => &f.bold,
    }
}

/// The width of `text` set in Inter at `size` px.
pub fn width(text: &str, size: f64, weight: Weight) -> f64 {
    let face = face(weight);
    let upem = f64::from(face.units_per_em());
    let fallback = face
        .glyph_index('?')
        .and_then(|g| face.glyph_hor_advance(g))
        .unwrap_or(0);
    let units: u32 = text
        .chars()
        .map(|c| {
            face.glyph_index(c)
                .and_then(|g| face.glyph_hor_advance(g))
                .unwrap_or(fallback) as u32
        })
        .sum();
    f64::from(units) * size / upem
}

/// Distance from the top of a line box to the baseline, at `size` px.
pub fn ascent(size: f64) -> f64 {
    let face = face(Weight::Regular);
    f64::from(face.ascender()) * size / f64::from(face.units_per_em())
}

/// Break `text` into lines no wider than `max` (hard breaks on newlines).
pub fn wrap(text: &str, max: f64, size: f64, weight: Weight) -> Vec<String> {
    let mut lines = Vec::new();
    for paragraph in text.split('\n') {
        let mut line = String::new();
        for word in paragraph.split(' ').filter(|w| !w.is_empty()) {
            let candidate = if line.is_empty() {
                word.to_string()
            } else {
                format!("{line} {word}")
            };
            if width(&candidate, size, weight) <= max {
                line = candidate;
                continue;
            }
            if !line.is_empty() {
                lines.push(std::mem::take(&mut line));
            }
            if width(word, size, weight) <= max {
                line = word.to_string();
            } else {
                // A word wider than the box: break it by character.
                for ch in word.chars() {
                    let next = format!("{line}{ch}");
                    if !line.is_empty() && width(&next, size, weight) > max {
                        lines.push(std::mem::take(&mut line));
                        line.push(ch);
                    } else {
                        line = next;
                    }
                }
            }
        }
        lines.push(line);
    }
    lines
}

#[derive(Debug, Clone, PartialEq)]
pub struct Fitted {
    pub size: f64,
    pub lines: Vec<String>,
}

/// Fit `text` in a `max_w` × `max_h` box: wrap at `size`, step the size down
/// to `min_size` while it overflows, then cut the last line with an ellipsis.
pub fn fit(
    text: &str,
    max_w: f64,
    max_h: f64,
    size: f64,
    min_size: f64,
    line_height: f64,
    weight: Weight,
) -> Fitted {
    let max_w = max_w.max(1.0);
    let mut s = size;
    loop {
        let lines = wrap(text, max_w, s, weight);
        if lines.len() as f64 * s * line_height <= max_h || s <= min_size {
            let mut lines = lines;
            let room = ((max_h / (s * line_height)).floor() as usize).max(1);
            if lines.len() > room {
                lines.truncate(room);
                let last = lines.last_mut().expect("at least one line");
                *last = ellipsize(last, max_w, s, weight);
            }
            return Fitted { size: s, lines };
        }
        s = (s - 1.0).max(min_size);
    }
}

/// `line` with "…" appended, shortened until it fits `max`.
fn ellipsize(line: &str, max: f64, size: f64, weight: Weight) -> String {
    let mut chars: Vec<char> = line.trim_end().chars().collect();
    loop {
        let candidate: String = chars.iter().collect::<String>() + "…";
        if chars.is_empty() || width(&candidate, size, weight) <= max {
            return candidate;
        }
        chars.pop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn measures_with_real_advances() {
        let narrow = width("iiii", 16.0, Weight::Regular);
        let wide = width("MMMM", 16.0, Weight::Regular);
        assert!(wide > narrow * 2.0, "Inter's M is much wider than i");
        assert!(width("Pay", 16.0, Weight::Bold) > width("Pay", 16.0, Weight::Regular));
        assert_eq!(
            width("Pay", 32.0, Weight::Regular),
            2.0 * width("Pay", 16.0, Weight::Regular)
        );
        assert_eq!(width("", 16.0, Weight::Regular), 0.0);
    }

    #[test]
    fn wraps_at_spaces_and_newlines() {
        let max = width("Choose a delivery", 15.0, Weight::Regular) + 0.5;
        let lines = wrap(
            "Choose a delivery slot\nthen pay",
            max,
            15.0,
            Weight::Regular,
        );
        assert_eq!(lines, vec!["Choose a delivery", "slot", "then pay"]);
    }

    #[test]
    fn breaks_words_wider_than_the_box() {
        let lines = wrap("Supercalifragilistic", 40.0, 15.0, Weight::Regular);
        assert!(lines.len() > 1);
        assert_eq!(lines.concat(), "Supercalifragilistic");
        assert!(
            lines
                .iter()
                .all(|l| width(l, 15.0, Weight::Regular) <= 40.0)
        );
    }

    #[test]
    fn steps_down_then_ellipsizes() {
        let short = fit("Pay", 140.0, 60.0, 15.0, 9.0, 1.25, Weight::SemiBold);
        assert_eq!(
            short,
            Fitted {
                size: 15.0,
                lines: vec!["Pay".into()]
            }
        );

        let long = "Check the delivery address against the postcode database before taking payment";
        let squeezed = fit(long, 140.0, 60.0, 15.0, 9.0, 1.25, Weight::SemiBold);
        assert!(squeezed.size < 15.0, "smaller text to fit");

        let hopeless = fit(
            &long.repeat(5),
            140.0,
            30.0,
            15.0,
            11.0,
            1.25,
            Weight::SemiBold,
        );
        assert_eq!(hopeless.size, 11.0);
        assert_eq!(hopeless.lines.len(), 2, "two lines of 11px fit in 30");
        assert!(hopeless.lines[1].ends_with('…'));
        assert!(width(&hopeless.lines[1], 11.0, Weight::SemiBold) <= 140.0);
    }
}
