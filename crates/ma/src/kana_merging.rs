use std::collections::HashMap;

/// Return the canonical char for `ch`: if the equiv map has an entry for it,
/// return that; otherwise return `ch` unchanged.
#[inline]
pub(crate) fn equiv_char(ch: char, equiv: &HashMap<char, char>) -> char {
    equiv.get(&ch).copied().unwrap_or(ch)
}

/// Build the halfwidth -> canonical char equivalence map.
///
/// When passed to [`equiv_char`] (or directly to the npz writers), every
/// halfwidth katakana (U+FF65..=U+FF9F) and fullwidth ASCII variant
/// (U+FF01..=U+FF5E) is transparently treated as its canonical counterpart
/// without mutating any records. Pass an empty map when no merging is wanted.
pub(crate) fn halfwidth_equiv() -> HashMap<char, char> {
    let mut m = HashMap::new();
    // Halfwidth katakana -> fullwidth katakana (FF65..=FF9F)
    let pairs: &[(char, char)] = &[
        ('\u{FF65}', '\u{30FB}'), // ･ -> ・
        ('\u{FF66}', '\u{30F2}'), // ｦ -> ヲ
        ('\u{FF67}', '\u{30A1}'), // ｧ -> ァ
        ('\u{FF68}', '\u{30A3}'), // ｨ -> ィ
        ('\u{FF69}', '\u{30A5}'), // ｩ -> ゥ
        ('\u{FF6A}', '\u{30A7}'), // ｪ -> ェ
        ('\u{FF6B}', '\u{30A9}'), // ｫ -> ォ
        ('\u{FF6C}', '\u{30E3}'), // ｬ -> ャ
        ('\u{FF6D}', '\u{30E5}'), // ｭ -> ュ
        ('\u{FF6E}', '\u{30E7}'), // ｮ -> ョ
        ('\u{FF6F}', '\u{30C3}'), // ｯ -> ッ
        ('\u{FF70}', '\u{30FC}'), // ｰ -> ー
        ('\u{FF71}', '\u{30A2}'), // ｱ -> ア
        ('\u{FF72}', '\u{30A4}'), // ｲ -> イ
        ('\u{FF73}', '\u{30A6}'), // ｳ -> ウ
        ('\u{FF74}', '\u{30A8}'), // ｴ -> エ
        ('\u{FF75}', '\u{30AA}'), // ｵ -> オ
        ('\u{FF76}', '\u{30AB}'), // ｶ -> カ
        ('\u{FF77}', '\u{30AD}'), // ｷ -> キ
        ('\u{FF78}', '\u{30AF}'), // ｸ -> ク
        ('\u{FF79}', '\u{30B1}'), // ｹ -> ケ
        ('\u{FF7A}', '\u{30B3}'), // ｺ -> コ
        ('\u{FF7B}', '\u{30B5}'), // ｻ -> サ
        ('\u{FF7C}', '\u{30B7}'), // ｼ -> シ
        ('\u{FF7D}', '\u{30B9}'), // ｽ -> ス
        ('\u{FF7E}', '\u{30BB}'), // ｾ -> セ
        ('\u{FF7F}', '\u{30BD}'), // ｿ -> ソ
        ('\u{FF80}', '\u{30BF}'), // ﾀ -> タ
        ('\u{FF81}', '\u{30C1}'), // ﾁ -> チ
        ('\u{FF82}', '\u{30C4}'), // ﾂ -> ツ
        ('\u{FF83}', '\u{30C6}'), // ﾃ -> テ
        ('\u{FF84}', '\u{30C8}'), // ﾄ -> ト
        ('\u{FF85}', '\u{30CA}'), // ﾅ -> ナ
        ('\u{FF86}', '\u{30CB}'), // ﾆ -> ニ
        ('\u{FF87}', '\u{30CC}'), // ﾇ -> ヌ
        ('\u{FF88}', '\u{30CD}'), // ﾈ -> ネ
        ('\u{FF89}', '\u{30CE}'), // ﾉ -> ノ
        ('\u{FF8A}', '\u{30CF}'), // ﾊ -> ハ
        ('\u{FF8B}', '\u{30D2}'), // ﾋ -> ヒ
        ('\u{FF8C}', '\u{30D5}'), // ﾌ -> フ
        ('\u{FF8D}', '\u{30D8}'), // ﾍ -> ヘ
        ('\u{FF8E}', '\u{30DB}'), // ﾎ -> ホ
        ('\u{FF8F}', '\u{30DE}'), // ﾏ -> マ
        ('\u{FF90}', '\u{30DF}'), // ﾐ -> ミ
        ('\u{FF91}', '\u{30E0}'), // ﾑ -> ム
        ('\u{FF92}', '\u{30E1}'), // ﾒ -> メ
        ('\u{FF93}', '\u{30E2}'), // ﾓ -> モ
        ('\u{FF94}', '\u{30E4}'), // ﾔ -> ヤ
        ('\u{FF95}', '\u{30E6}'), // ﾕ -> ユ
        ('\u{FF96}', '\u{30E8}'), // ﾖ -> ヨ
        ('\u{FF97}', '\u{30E9}'), // ﾗ -> ラ
        ('\u{FF98}', '\u{30EA}'), // ﾘ -> リ
        ('\u{FF99}', '\u{30EB}'), // ﾙ -> ル
        ('\u{FF9A}', '\u{30EC}'), // ﾚ -> レ
        ('\u{FF9B}', '\u{30ED}'), // ﾛ -> ロ
        ('\u{FF9C}', '\u{30EF}'), // ﾜ -> ワ
        ('\u{FF9D}', '\u{30F3}'), // ﾝ -> ン
        ('\u{FF9E}', '\u{309B}'), // ﾞ -> ゛
        ('\u{FF9F}', '\u{309C}'), // ﾟ -> ゜
    ];
    for &(hw, fw) in pairs {
        m.insert(hw, fw);
    }
    // Fullwidth ASCII variants -> ASCII (FF01..=FF5E, offset 0xFEE0)
    for cp in 0xFF01u32..=0xFF5Eu32 {
        let hw = char::from_u32(cp).unwrap();
        let ascii = char::from_u32(cp - 0xFEE0).unwrap();
        m.insert(hw, ascii);
    }
    m
}

/// Merge a single halfwidth katakana character to its fullwidth/standard form.
///
/// This is the single-char entry point used by tests. Production code builds
/// the equiv map via [`halfwidth_equiv`] and resolves characters through
/// [`equiv_char`] instead of calling this directly.
#[cfg_attr(not(test), allow(dead_code))]
///
/// When `merge` is true, halfwidth katakana (U+FF65..U+FF9F) are mapped to
/// their standard fullwidth katakana equivalents, and fullwidth ASCII variants
/// (U+FF01..U+FF5E) are mapped back to plain ASCII.
///
/// Returns the merged character, or the original if no merge applies.
pub(crate) fn merge_halfwidth(ch: char, merge: bool) -> char {
    if !merge {
        return ch;
    }

    // Check Unicode name to determine if merging is needed
    let name = match unicode_names2::name(ch) {
        Some(n) => n,
        None => return ch,
    };

    let name_str = name.to_string();
    let name_upper = name_str.to_uppercase();

    // Only process if the name contains HALFWIDTH or FULLWIDTH
    if !name_upper.contains("HALFWIDTH") && !name_upper.contains("FULLWIDTH") {
        return ch;
    }

    match ch {
        // Halfwidth Katakana -> fullwidth Katakana
        // U+FF65..=U+FF9F: character-specific mapping per Unicode standard
        '\u{FF65}'..='\u{FF9F}' => match ch {
            '\u{FF65}' => '\u{30FB}', // Middle Dot          ･ -> ・
            '\u{FF66}' => '\u{30F2}', // Wo                  ｦ -> ヲ
            '\u{FF67}' => '\u{30A1}', // Small A             ｧ -> ァ
            '\u{FF68}' => '\u{30A3}', // Small I             ｨ -> ィ
            '\u{FF69}' => '\u{30A5}', // Small U             ｩ -> ゥ
            '\u{FF6A}' => '\u{30A7}', // Small E             ｪ -> ェ
            '\u{FF6B}' => '\u{30A9}', // Small O             ｫ -> ォ
            '\u{FF6C}' => '\u{30E3}', // Small Ya            ｬ -> ャ
            '\u{FF6D}' => '\u{30E5}', // Small Yu            ｭ -> ュ
            '\u{FF6E}' => '\u{30E7}', // Small Yo            ｮ -> ョ
            '\u{FF6F}' => '\u{30C3}', // Small Tu            ｯ -> ッ
            '\u{FF70}' => '\u{30FC}', // Prolonged Sound Mark ｰ -> ー
            '\u{FF71}' => '\u{30A2}', // A                   ｱ -> ア
            '\u{FF72}' => '\u{30A4}', // I                   ｲ -> イ
            '\u{FF73}' => '\u{30A6}', // U                   ｳ -> ウ
            '\u{FF74}' => '\u{30A8}', // E                   ｴ -> エ
            '\u{FF75}' => '\u{30AA}', // O                   ｵ -> オ
            '\u{FF76}' => '\u{30AB}', // Ka                  ｶ -> カ
            '\u{FF77}' => '\u{30AD}', // Ki                  ｷ -> キ
            '\u{FF78}' => '\u{30AF}', // Ku                  ｸ -> ク
            '\u{FF79}' => '\u{30B1}', // Ke                  ｹ -> ケ
            '\u{FF7A}' => '\u{30B3}', // Ko                  ｺ -> コ
            '\u{FF7B}' => '\u{30B5}', // Sa                  ｻ -> サ
            '\u{FF7C}' => '\u{30B7}', // Si                  ｼ -> シ
            '\u{FF7D}' => '\u{30B9}', // Su                  ｽ -> ス
            '\u{FF7E}' => '\u{30BB}', // Se                  ｾ -> セ
            '\u{FF7F}' => '\u{30BD}', // So                  ｿ -> ソ
            '\u{FF80}' => '\u{30BF}', // Ta                  ﾀ -> タ
            '\u{FF81}' => '\u{30C1}', // Ti                  ﾁ -> チ
            '\u{FF82}' => '\u{30C4}', // Tu                  ﾂ -> ツ
            '\u{FF83}' => '\u{30C6}', // Te                  ﾃ -> テ
            '\u{FF84}' => '\u{30C8}', // To                  ﾄ -> ト
            '\u{FF85}' => '\u{30CA}', // Na                  ﾅ -> ナ
            '\u{FF86}' => '\u{30CB}', // Ni                  ﾆ -> ニ
            '\u{FF87}' => '\u{30CC}', // Nu                  ﾇ -> ヌ
            '\u{FF88}' => '\u{30CD}', // Ne                  ﾈ -> ネ
            '\u{FF89}' => '\u{30CE}', // No                  ﾉ -> ノ
            '\u{FF8A}' => '\u{30CF}', // Ha                  ﾊ -> ハ
            '\u{FF8B}' => '\u{30D2}', // Hi                  ﾋ -> ヒ
            '\u{FF8C}' => '\u{30D5}', // Hu                  ﾌ -> フ
            '\u{FF8D}' => '\u{30D8}', // He                  ﾍ -> ヘ
            '\u{FF8E}' => '\u{30DB}', // Ho                  ﾎ -> ホ
            '\u{FF8F}' => '\u{30DE}', // Ma                  ﾏ -> マ
            '\u{FF90}' => '\u{30DF}', // Mi                  ﾐ -> ミ
            '\u{FF91}' => '\u{30E0}', // Mu                  ﾑ -> ム
            '\u{FF92}' => '\u{30E1}', // Me                  ﾒ -> メ
            '\u{FF93}' => '\u{30E2}', // Mo                  ﾓ -> モ
            '\u{FF94}' => '\u{30E4}', // Ya                  ﾔ -> ヤ
            '\u{FF95}' => '\u{30E6}', // Yu                  ﾕ -> ユ
            '\u{FF96}' => '\u{30E8}', // Yo                  ﾖ -> ヨ
            '\u{FF97}' => '\u{30E9}', // Ra                  ﾗ -> ラ
            '\u{FF98}' => '\u{30EA}', // Ri                  ﾘ -> リ
            '\u{FF99}' => '\u{30EB}', // Ru                  ﾙ -> ル
            '\u{FF9A}' => '\u{30EC}', // Re                  ﾚ -> レ
            '\u{FF9B}' => '\u{30ED}', // Ro                  ﾛ -> ロ
            '\u{FF9C}' => '\u{30EF}', // Wa                  ﾜ -> ワ
            '\u{FF9D}' => '\u{30F3}', // N                   ﾝ -> ン
            '\u{FF9E}' => '\u{309B}', // Voiced Sound Mark   ﾞ -> ゛
            '\u{FF9F}' => '\u{309C}', // Semi-Voiced Sound Mark ﾟ -> ゜
            _ => ch,
        },
        // Fullwidth ASCII variants -> ASCII
        // U+FF01..=U+FF5E -> U+0021..=U+007E (offset: 0xFEE0)
        '\u{FF01}'..='\u{FF5E}' => {
            let mapped = ch as u32 - 0xFEE0;
            char::from_u32(mapped).unwrap_or(ch)
        }
        _ => ch,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // All 35 halfwidth katakana chars (FF65–FF9F) mapped to their correct
    // fullwidth equivalents per the Unicode standard.
    #[test]
    fn merge_halfwidth_all_katakana() {
        let cases: &[(char, char)] = &[
            ('\u{FF65}', '\u{30FB}'), // ･ -> ・ Middle Dot
            ('\u{FF66}', '\u{30F2}'), // ｦ -> ヲ Wo
            ('\u{FF67}', '\u{30A1}'), // ｧ -> ァ Small A
            ('\u{FF68}', '\u{30A3}'), // ｨ -> ィ Small I
            ('\u{FF69}', '\u{30A5}'), // ｩ -> ゥ Small U
            ('\u{FF6A}', '\u{30A7}'), // ｪ -> ェ Small E
            ('\u{FF6B}', '\u{30A9}'), // ｫ -> ォ Small O
            ('\u{FF6C}', '\u{30E3}'), // ｬ -> ャ Small Ya
            ('\u{FF6D}', '\u{30E5}'), // ｭ -> ュ Small Yu
            ('\u{FF6E}', '\u{30E7}'), // ｮ -> ョ Small Yo
            ('\u{FF6F}', '\u{30C3}'), // ｯ -> ッ Small Tu
            ('\u{FF70}', '\u{30FC}'), // ｰ -> ー Prolonged Sound Mark
            ('\u{FF71}', '\u{30A2}'), // ｱ -> ア A
            ('\u{FF72}', '\u{30A4}'), // ｲ -> イ I
            ('\u{FF73}', '\u{30A6}'), // ｳ -> ウ U
            ('\u{FF74}', '\u{30A8}'), // ｴ -> エ E
            ('\u{FF75}', '\u{30AA}'), // ｵ -> オ O
            ('\u{FF76}', '\u{30AB}'), // ｶ -> カ Ka
            ('\u{FF77}', '\u{30AD}'), // ｷ -> キ Ki
            ('\u{FF78}', '\u{30AF}'), // ｸ -> ク Ku
            ('\u{FF79}', '\u{30B1}'), // ｹ -> ケ Ke
            ('\u{FF7A}', '\u{30B3}'), // ｺ -> コ Ko
            ('\u{FF7B}', '\u{30B5}'), // ｻ -> サ Sa
            ('\u{FF7C}', '\u{30B7}'), // ｼ -> シ Si
            ('\u{FF7D}', '\u{30B9}'), // ｽ -> ス Su
            ('\u{FF7E}', '\u{30BB}'), // ｾ -> セ Se
            ('\u{FF7F}', '\u{30BD}'), // ｿ -> ソ So
            ('\u{FF80}', '\u{30BF}'), // ﾀ -> タ Ta
            ('\u{FF81}', '\u{30C1}'), // ﾁ -> チ Ti
            ('\u{FF82}', '\u{30C4}'), // ﾂ -> ツ Tu
            ('\u{FF83}', '\u{30C6}'), // ﾃ -> テ Te
            ('\u{FF84}', '\u{30C8}'), // ﾄ -> ト To
            ('\u{FF85}', '\u{30CA}'), // ﾅ -> ナ Na
            ('\u{FF86}', '\u{30CB}'), // ﾆ -> ニ Ni
            ('\u{FF87}', '\u{30CC}'), // ﾇ -> ヌ Nu
            ('\u{FF88}', '\u{30CD}'), // ﾈ -> ネ Ne
            ('\u{FF89}', '\u{30CE}'), // ﾉ -> ノ No
            ('\u{FF8A}', '\u{30CF}'), // ﾊ -> ハ Ha
            ('\u{FF8B}', '\u{30D2}'), // ﾋ -> ヒ Hi
            ('\u{FF8C}', '\u{30D5}'), // ﾌ -> フ Hu
            ('\u{FF8D}', '\u{30D8}'), // ﾍ -> ヘ He
            ('\u{FF8E}', '\u{30DB}'), // ﾎ -> ホ Ho
            ('\u{FF8F}', '\u{30DE}'), // ﾏ -> マ Ma
            ('\u{FF90}', '\u{30DF}'), // ﾐ -> ミ Mi
            ('\u{FF91}', '\u{30E0}'), // ﾑ -> ム Mu
            ('\u{FF92}', '\u{30E1}'), // ﾒ -> メ Me
            ('\u{FF93}', '\u{30E2}'), // ﾓ -> モ Mo
            ('\u{FF94}', '\u{30E4}'), // ﾔ -> ヤ Ya
            ('\u{FF95}', '\u{30E6}'), // ﾕ -> ユ Yu
            ('\u{FF96}', '\u{30E8}'), // ﾖ -> ヨ Yo
            ('\u{FF97}', '\u{30E9}'), // ﾗ -> ラ Ra
            ('\u{FF98}', '\u{30EA}'), // ﾘ -> リ Ri
            ('\u{FF99}', '\u{30EB}'), // ﾙ -> ル Ru
            ('\u{FF9A}', '\u{30EC}'), // ﾚ -> レ Re
            ('\u{FF9B}', '\u{30ED}'), // ﾛ -> ロ Ro
            ('\u{FF9C}', '\u{30EF}'), // ﾜ -> ワ Wa
            ('\u{FF9D}', '\u{30F3}'), // ﾝ -> ン N
            ('\u{FF9E}', '\u{309B}'), // ﾞ -> ゛ Voiced Sound Mark
            ('\u{FF9F}', '\u{309C}'), // ﾟ -> ゜ Semi-Voiced Sound Mark
        ];
        for &(input, expected) in cases {
            let result = merge_halfwidth(input, true);
            assert_eq!(
                result, expected,
                "halfwidth U+{:04X} ({}) should map to U+{:04X} ({}), got U+{:04X} ({})",
                input as u32, input, expected as u32, expected, result as u32, result
            );
        }
    }

    #[test]
    fn merge_disabled_returns_original() {
        // With merge=false every halfwidth char comes back unchanged.
        let hw_chars = [
            '\u{FF65}', '\u{FF6C}', '\u{FF71}', '\u{FF82}', '\u{FF83}', '\u{FF9F}',
        ];
        for ch in hw_chars {
            assert_eq!(
                merge_halfwidth(ch, false),
                ch,
                "merge=false should return U+{:04X} unchanged",
                ch as u32
            );
        }
    }

    #[test]
    fn merge_fullwidth_ascii_to_ascii() {
        // U+FF01..=U+FF5E fullwidth ASCII variants should strip back to plain ASCII.
        let cases: &[(char, char)] = &[
            ('\u{FF01}', '!'), // ！-> !
            ('\u{FF10}', '0'), // ０-> 0
            ('\u{FF21}', 'A'), // Ａ-> A
            ('\u{FF41}', 'a'), // ａ-> a
            ('\u{FF5E}', '~'), // ～-> ~
        ];
        for &(input, expected) in cases {
            let result = merge_halfwidth(input, true);
            assert_eq!(
                result, expected,
                "fullwidth ASCII U+{:04X} should map to U+{:04X}",
                input as u32, expected as u32
            );
        }
    }

    #[test]
    fn halfwidth_equiv_map_size_and_spot_checks() {
        let m = halfwidth_equiv();
        // 59 halfwidth katakana (FF65..=FF9F) + 94 fullwidth ASCII (FF01..=FF5E)
        assert_eq!(m.len(), 59 + 94, "equiv map should have 153 entries");

        // Spot-check regular katakana
        assert_eq!(m[&'\u{FF71}'], '\u{30A2}', "ｱ -> ア");
        assert_eq!(m[&'\u{FF83}'], '\u{30C6}', "ﾃ -> テ");
        assert_eq!(m[&'\u{FF84}'], '\u{30C8}', "ﾄ -> ト");
        assert_eq!(m[&'\u{FF9F}'], '\u{309C}', "ﾟ -> ゜");

        // Verify previously-buggy mappings now point to the LARGE (non-small) forms
        assert_eq!(m[&'\u{FF82}'], '\u{30C4}', "ﾂ -> ツ (not ッ SMALL TU)");
        assert_eq!(m[&'\u{FF94}'], '\u{30E4}', "ﾔ -> ヤ (not ャ SMALL YA)");
        assert_eq!(m[&'\u{FF95}'], '\u{30E6}', "ﾕ -> ユ (not ュ SMALL YU)");
        assert_eq!(m[&'\u{FF96}'], '\u{30E8}', "ﾖ -> ヨ (not ョ SMALL YO)");

        // Small forms must still come from their proper halfwidth small origins
        assert_eq!(m[&'\u{FF6C}'], '\u{30E3}', "ｬ -> ャ SMALL YA");
        assert_eq!(m[&'\u{FF6D}'], '\u{30E5}', "ｭ -> ュ SMALL YU");
        assert_eq!(m[&'\u{FF6E}'], '\u{30E7}', "ｮ -> ョ SMALL YO");
        assert_eq!(m[&'\u{FF6F}'], '\u{30C3}', "ｯ -> ッ SMALL TU");

        // Spot-check fullwidth ASCII
        assert_eq!(m[&'\u{FF01}'], '!');
        assert_eq!(m[&'\u{FF41}'], 'a');
    }

    #[test]
    fn equiv_char_uses_map_and_falls_through() {
        let m = halfwidth_equiv();
        // Mapped char
        assert_eq!(equiv_char('\u{FF71}', &m), '\u{30A2}');
        // Unmapped char passes through
        assert_eq!(equiv_char('ア', &m), 'ア');
        assert_eq!(equiv_char('a', &m), 'a');
    }

    #[test]
    fn merge_non_halfwidth_returned_unchanged() {
        // Regular fullwidth katakana and ASCII should pass through untouched.
        let plain = ['ア', 'テ', 'ト', 'a', '!', '\u{3042}'];
        for ch in plain {
            assert_eq!(
                merge_halfwidth(ch, true),
                ch,
                "non-halfwidth char U+{:04X} ({}) should be unchanged",
                ch as u32,
                ch
            );
        }
    }
}
