//! The web page keeps to its design system (specs/2026-11-28-design-system.md, Part 11): colours,
//! sizes, corners and shadows come from the token block at the top of the `<style>`, the layout
//! changes at three widths only, and inline `style="…"` attributes only ever go down.
//!
//! It reads `src/ui/index.html` as text; nothing runs.

use regex::Regex;

const PAGE: &str = include_str!("../src/ui/index.html");

/// Inline `style="` attributes allowed in the page. A ratchet: lower it as screens are converted,
/// never raise it. All that is left are values computed in JS (area colours).
const INLINE_STYLE_CEILING: usize = 8;

fn style_block() -> &'static str {
    let a = PAGE.find("<style>").expect("the page has a <style>") + "<style>".len();
    let b = PAGE.find("</style>").expect("the page has a </style>");
    &PAGE[a..b]
}

/// The `:root{…}` token block: byte range within the whole page.
fn root_block() -> (usize, usize) {
    let css_at = PAGE.find("<style>").unwrap();
    let a = css_at + PAGE[css_at..].find(":root{").expect("the <style> starts with a :root token block");
    let b = a + PAGE[a..].find("\n}").expect("the token block ends with a } on its own line") + 2;
    (a, b)
}

fn line_of(at: usize) -> usize {
    PAGE[..at].matches('\n').count() + 1
}

/// The CSS without comments, so a value named in a comment is not taken for a rule.
fn css_rules() -> String {
    Regex::new(r"(?s)/\*.*?\*/").unwrap().replace_all(style_block(), "").into_owned()
}

#[test]
fn hex_colours_live_only_in_the_token_block() {
    let (ra, rb) = root_block();
    let hex = Regex::new(r"#([0-9a-fA-F]{3,8})(?:[^0-9A-Za-z_-]|$)").unwrap();
    let mut found = Vec::new();
    for m in hex.captures_iter(PAGE) {
        let g = m.get(1).unwrap();
        if ![3, 4, 6, 8].contains(&g.as_str().len()) {
            continue;
        }
        let at = g.start() - 1;
        if at >= ra && at < rb {
            continue;
        }
        found.push(format!("line {}: #{}", line_of(at), g.as_str()));
    }
    assert!(found.is_empty(), "raw hex colours outside :root (use a var(--…) token):\n{}", found.join("\n"));
}

#[test]
fn sizes_corners_and_shadows_are_tokens() {
    let css = css_rules();
    let root_end = css.find("\n}").unwrap() + 2;
    let decl = Regex::new(r"(?:^|[;{\s])(font-size|border-radius|box-shadow|font)\s*:\s*([^;}]+)").unwrap();
    let var = Regex::new(r"var\(--[\w-]+\)").unwrap();
    let raw = Regex::new(r"\d*\.?\d+(px|em|rem|%|pt|vh|vw|lh)\b").unwrap();
    let mut found = Vec::new();
    for c in decl.captures_iter(&css[root_end..]) {
        let (prop, value) = (&c[1], c[2].trim());
        let left = var.replace_all(value, "");
        if raw.is_match(&left) {
            found.push(format!("{}: {}", prop, value));
        }
    }
    assert!(found.is_empty(), "raw sizes in the CSS (use --fs-*, --r-*, --shadow-* tokens):\n{}", found.join("\n"));
}

#[test]
fn media_queries_use_the_three_tier_widths() {
    let css = css_rules();
    let media = Regex::new(r"@media[^{]*").unwrap();
    let num = Regex::new(r"(\d+(?:\.\d+)?)px").unwrap();
    let mut found = Vec::new();
    for m in media.find_iter(&css) {
        for n in num.captures_iter(m.as_str()) {
            if !["720", "1024", "1280"].contains(&&n[1]) {
                found.push(m.as_str().trim().to_string());
            }
        }
    }
    assert!(found.is_empty(), "media queries may only use 720, 1024 and 1280 px:\n{}", found.join("\n"));
    // the JS knows the same three widths, in one place
    assert!(
        PAGE.contains("const TIERS = { tablet: 720, laptop: 1024, desktop: 1280 };"),
        "the JS's TIERS must match the CSS's breakpoints"
    );
    assert!(!PAGE.contains("innerWidth < 760") && !PAGE.contains("innerWidth < 1100"), "an old breakpoint is back in the JS");
}

#[test]
fn every_var_used_is_a_token() {
    let (ra, rb) = root_block();
    let def = Regex::new(r"(--[\w-]+)\s*:").unwrap();
    let defined: std::collections::HashSet<&str> = def.captures_iter(&PAGE[ra..rb]).map(|c| c.get(1).unwrap().as_str()).collect();
    let used = Regex::new(r"var\((--[\w-]+)").unwrap();
    let mut missing: Vec<String> = used
        .captures_iter(PAGE)
        .map(|c| c.get(1).unwrap().as_str())
        .filter(|n| !defined.contains(n))
        .map(|n| n.to_string())
        .collect();
    missing.sort();
    missing.dedup();
    assert!(missing.is_empty(), "var(--…) names that the token block doesn't define (retired or misspelt): {:?}", missing);
}

#[test]
fn inline_styles_only_go_down() {
    let n = PAGE.matches("style=\"").count();
    assert!(
        n <= INLINE_STYLE_CEILING,
        "{} inline style=\"…\" attributes, over the ceiling of {}: use a class (design system Part 11)",
        n,
        INLINE_STYLE_CEILING
    );
}
