use std::path::{Path, PathBuf};

pub fn path_to_string<P: AsRef<Path>>(path: P) -> String {
    path.as_ref()
        .to_path_buf()
        .into_os_string()
        .into_string()
        .unwrap_or_else(|os_string| os_string.to_string_lossy().into())
}

fn app_dir_name() -> &'static str {
    #[cfg(debug_assertions)]
    {
        "kimun_debug"
    }
    #[cfg(not(debug_assertions))]
    {
        "kimun"
    }
}

/// Returns the platform-specific directory where Kimün stores its log file.
///
/// Fallback chain per platform:
///   preferred dir → `current_dir()` → `temp_dir()` (always available)
/// Always returns an absolute path with the app-name suffix appended.
pub fn app_log_dir() -> PathBuf {
    let name = app_dir_name();
    let fallback = || {
        std::env::current_dir().unwrap_or_else(|_| std::env::temp_dir())
    };
    #[cfg(target_os = "macos")]
    {
        std::env::var("HOME")
            .map(|h| PathBuf::from(h).join("Library/Application Support").join(name))
            .unwrap_or_else(|_| fallback().join(name))
    }
    #[cfg(target_os = "linux")]
    {
        std::env::var("XDG_DATA_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                std::env::var("HOME")
                    .map(|h| PathBuf::from(h).join(".local/share"))
                    .unwrap_or_else(|_| fallback())
            })
            .join(name)
    }
    #[cfg(target_os = "windows")]
    {
        std::env::var("APPDATA")
            .map(|p| PathBuf::from(p).join(name))
            .unwrap_or_else(|_| fallback())
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        fallback().join(name)
    }
}

// taken from https://github.com/YesSeri/diacritics/
// with modifications
pub fn remove_diacritics(string: &str) -> String {
    let chars = string.chars();
    chars.fold(String::with_capacity(string.len()), |mut acc, current| {
        escape_diacritic(&mut acc, current);
        acc
    })
}

fn escape_diacritic(acc: &mut String, current: char) {
    match current {
        'A' | 'Ⓐ' | 'Ａ' | 'À' | 'Á' | 'Â' | 'Ầ' | 'Ấ' | 'Ẫ' | 'Ẩ' | 'Ã' | 'Ā' | 'Ă' | 'Ằ'
        | 'Ắ' | 'Ẵ' | 'Ẳ' | 'Ȧ' | 'Ǡ' | 'Ä' | 'Ǟ' | 'Ả' | 'Å' | 'Ǻ' | 'Ǎ' | 'Ȁ' | 'Ȃ' | 'Ạ'
        | 'Ậ' | 'Ặ' | 'Ḁ' | 'Ą' | 'Ⱥ' | 'Ɐ' => acc.push('A'),
        'Ꜳ' => acc.push_str("AA"),
        'Æ' | 'Ǽ' | 'Ǣ' => acc.push('A'),
        'Ꜵ' => acc.push_str("AO"),
        'Ꜷ' => acc.push_str("AU"),
        'Ꜹ' | 'Ꜻ' => acc.push_str("AV"),
        'Ꜽ' => acc.push_str("AY"),
        'B' | 'Ⓑ' | 'Ｂ' | 'Ḃ' | 'Ḅ' | 'Ḇ' | 'Ƀ' | 'Ƃ' | 'Ɓ' => acc.push('B'),
        'C' | 'Ⓒ' | 'Ｃ' | 'Ć' | 'Ĉ' | 'Ċ' | 'Č' | 'Ç' | 'Ḉ' | 'Ƈ' | 'Ȼ' | 'Ꜿ' => {
            acc.push('C')
        }
        'D' | 'Ⓓ' | 'Ｄ' | 'Ḋ' | 'Ď' | 'Ḍ' | 'Ḑ' | 'Ḓ' | 'Ḏ' | 'Đ' | 'Ƌ' | 'Ɗ' | 'Ɖ' | 'Ꝺ' => {
            acc.push('D')
        }
        'Ǳ' | 'Ǆ' => acc.push_str("DZ"),
        'ǲ' | 'ǅ' => acc.push_str("Dz"),
        'E' | 'Ⓔ' | 'Ｅ' | 'È' | 'É' | 'Ê' | 'Ề' | 'Ế' | 'Ễ' | 'Ể' | 'Ẽ' | 'Ē' | 'Ḕ' | 'Ḗ'
        | 'Ĕ' | 'Ė' | 'Ë' | 'Ẻ' | 'Ě' | 'Ȅ' | 'Ȇ' | 'Ẹ' | 'Ệ' | 'Ȩ' | 'Ḝ' | 'Ę' | 'Ḙ' | 'Ḛ'
        | 'Ɛ' | 'Ǝ' => acc.push('E'),
        'F' | 'Ⓕ' | 'Ｆ' | 'Ḟ' | 'Ƒ' | 'Ꝼ' => acc.push('F'),
        'G' | 'Ⓖ' | 'Ｇ' | 'Ǵ' | 'Ĝ' | 'Ḡ' | 'Ğ' | 'Ġ' | 'Ǧ' | 'Ģ' | 'Ǥ' | 'Ɠ' | 'Ꞡ' | 'Ᵹ'
        | 'Ꝿ' => acc.push('G'),
        'H' | 'Ⓗ' | 'Ｈ' | 'Ĥ' | 'Ḣ' | 'Ḧ' | 'Ȟ' | 'Ḥ' | 'Ḩ' | 'Ḫ' | 'Ħ' | 'Ⱨ' | 'Ⱶ' | 'Ɥ' => {
            acc.push('H')
        }
        'I' | 'Ⓘ' | 'Ｉ' | 'Ì' | 'Í' | 'Î' | 'Ĩ' | 'Ī' | 'Ĭ' | 'İ' | 'Ï' | 'Ḯ' | 'Ỉ' | 'Ǐ'
        | 'Ȉ' | 'Ȋ' | 'Ị' | 'Į' | 'Ḭ' | 'Ɨ' => acc.push('I'),
        'J' | 'Ⓙ' | 'Ｊ' | 'Ĵ' | 'Ɉ' => acc.push('J'),
        'K' | 'Ⓚ' | 'Ｋ' | 'Ḱ' | 'Ǩ' | 'Ḳ' | 'Ķ' | 'Ḵ' | 'Ƙ' | 'Ⱪ' | 'Ꝁ' | 'Ꝃ' | 'Ꝅ' | 'Ꞣ' => {
            acc.push('K')
        }
        'L' | 'Ⓛ' | 'Ｌ' | 'Ŀ' | 'Ĺ' | 'Ľ' | 'Ḷ' | 'Ḹ' | 'Ļ' | 'Ḽ' | 'Ḻ' | 'Ł' | 'Ƚ' | 'Ɫ'
        | 'Ⱡ' | 'Ꝉ' | 'Ꝇ' | 'Ꞁ' => acc.push('L'),
        'Ǉ' => acc.push_str("LJ"),
        'ǈ' => acc.push_str("Lj"),
        'M' | 'Ⓜ' | 'Ｍ' | 'Ḿ' | 'Ṁ' | 'Ṃ' | 'Ɱ' | 'Ɯ' => acc.push('M'),
        'N' | 'Ⓝ' | 'Ｎ' | 'Ǹ' | 'Ń' | 'Ñ' | 'Ṅ' | 'Ň' | 'Ṇ' | 'Ņ' | 'Ṋ' | 'Ṉ' | 'Ƞ' | 'Ɲ'
        | 'Ꞑ' | 'Ꞥ' => acc.push('N'),
        'Ǌ' => acc.push_str("NJ"),
        'ǋ' => acc.push_str("Nj"),
        'O' | 'Ⓞ' | 'Ｏ' | 'Ò' | 'Ó' | 'Ô' | 'Ồ' | 'Ố' | 'Ỗ' | 'Ổ' | 'Õ' | 'Ṍ' | 'Ȭ' | 'Ṏ'
        | 'Ō' | 'Ṑ' | 'Ṓ' | 'Ŏ' | 'Ȯ' | 'Ȱ' | 'Ö' | 'Ȫ' | 'Ỏ' | 'Ő' | 'Ǒ' | 'Ȍ' | 'Ȏ' | 'Ơ'
        | 'Ờ' | 'Ớ' | 'Ỡ' | 'Ở' | 'Ợ' | 'Ọ' | 'Ộ' | 'Ǫ' | 'Ǭ' | 'Ø' | 'Ǿ' | 'Ɔ' | 'Ɵ' | 'Ꝋ'
        | 'Ꝍ' => acc.push('O'),
        'Ƣ' => acc.push_str("OI"),
        'Ꝏ' => acc.push_str("OO"),
        'Ȣ' => acc.push_str("OU"),
        '\u{008C}' | 'Œ' => acc.push_str("OE"),
        '\u{009C}' | 'œ' => acc.push_str("oe"),
        'P' | 'Ⓟ' | 'Ｐ' | 'Ṕ' | 'Ṗ' | 'Ƥ' | 'Ᵽ' | 'Ꝑ' | 'Ꝓ' | 'Ꝕ' => acc.push('P'),
        'Q' | 'Ⓠ' | 'Ｑ' | 'Ꝗ' | 'Ꝙ' | 'Ɋ' => acc.push('Q'),
        'R' | 'Ⓡ' | 'Ｒ' | 'Ŕ' | 'Ṙ' | 'Ř' | 'Ȑ' | 'Ȓ' | 'Ṛ' | 'Ṝ' | 'Ŗ' | 'Ṟ' | 'Ɍ' | 'Ɽ'
        | 'Ꝛ' | 'Ꞧ' | 'Ꞃ' => acc.push('R'),
        'S' | 'Ⓢ' | 'Ｓ' | 'ẞ' | 'Ś' | 'Ṥ' | 'Ŝ' | 'Ṡ' | 'Š' | 'Ṧ' | 'Ṣ' | 'Ṩ' | 'Ș' | 'Ş'
        | 'Ȿ' | 'Ꞩ' | 'Ꞅ' => acc.push('S'),
        'T' | 'Ⓣ' | 'Ｔ' | 'Ṫ' | 'Ť' | 'Ṭ' | 'Ț' | 'Ţ' | 'Ṱ' | 'Ṯ' | 'Ŧ' | 'Ƭ' | 'Ʈ' | 'Ⱦ'
        | 'Ꞇ' => acc.push('T'),
        'Ꜩ' => acc.push_str("TZ"),
        'U' | 'Ⓤ' | 'Ｕ' | 'Ù' | 'Ú' | 'Û' | 'Ũ' | 'Ṹ' | 'Ū' | 'Ṻ' | 'Ŭ' | 'Ü' | 'Ǜ' | 'Ǘ'
        | 'Ǖ' | 'Ǚ' | 'Ủ' | 'Ů' | 'Ű' | 'Ǔ' | 'Ȕ' | 'Ȗ' | 'Ư' | 'Ừ' | 'Ứ' | 'Ữ' | 'Ử' | 'Ự'
        | 'Ụ' | 'Ṳ' | 'Ų' | 'Ṷ' | 'Ṵ' | 'Ʉ' => acc.push('U'),
        'V' | 'Ⓥ' | 'Ｖ' | 'Ṽ' | 'Ṿ' | 'Ʋ' | 'Ꝟ' | 'Ʌ' => acc.push('V'),
        'Ꝡ' => acc.push_str("VY"),
        'W' | 'Ⓦ' | 'Ｗ' | 'Ẁ' | 'Ẃ' | 'Ŵ' | 'Ẇ' | 'Ẅ' | 'Ẉ' | 'Ⱳ' => acc.push('W'),
        'X' | 'Ⓧ' | 'Ｘ' | 'Ẋ' | 'Ẍ' => acc.push('X'),
        'Y' | 'Ⓨ' | 'Ｙ' | 'Ỳ' | 'Ý' | 'Ŷ' | 'Ỹ' | 'Ȳ' | 'Ẏ' | 'Ÿ' | 'Ỷ' | 'Ỵ' | 'Ƴ' | 'Ɏ'
        | 'Ỿ' => acc.push('Y'),
        'Z' | 'Ⓩ' | 'Ｚ' | 'Ź' | 'Ẑ' | 'Ż' | 'Ž' | 'Ẓ' | 'Ẕ' | 'Ƶ' | 'Ȥ' | 'Ɀ' | 'Ⱬ' | 'Ꝣ' => {
            acc.push('Z')
        }
        'a' | 'ⓐ' | 'ａ' | 'ẚ' | 'à' | 'á' | 'â' | 'ầ' | 'ấ' | 'ẫ' | 'ẩ' | 'ã' | 'ā' | 'ă'
        | 'ằ' | 'ắ' | 'ẵ' | 'ẳ' | 'ȧ' | 'ǡ' | 'ä' | 'ǟ' | 'ả' | 'å' | 'ǻ' | 'ǎ' | 'ȁ' | 'ȃ'
        | 'ạ' | 'ậ' | 'ặ' | 'ḁ' | 'ą' | 'ⱥ' | 'ɐ' => acc.push('a'),
        'ꜳ' => acc.push_str("aa"),
        'æ' | 'ǽ' | 'ǣ' => acc.push('a'),
        'ꜵ' => acc.push_str("ao"),
        'ꜷ' => acc.push_str("au"),
        'ꜹ' | 'ꜻ' => acc.push_str("av"),
        'ꜽ' => acc.push_str("ay"),
        'b' | 'ⓑ' | 'ｂ' | 'ḃ' | 'ḅ' | 'ḇ' | 'ƀ' | 'ƃ' | 'ɓ' | 'þ' => acc.push('b'),
        'c' | 'ⓒ' | 'ｃ' | 'ć' | 'ĉ' | 'ċ' | 'č' | 'ç' | 'ḉ' | 'ƈ' | 'ȼ' | 'ꜿ' | 'ↄ' => {
            acc.push('c')
        }
        'd' | 'ⓓ' | 'ｄ' | 'ḋ' | 'ď' | 'ḍ' | 'ḑ' | 'ḓ' | 'ḏ' | 'đ' | 'ƌ' | 'ɖ' | 'ɗ' | 'ꝺ' => {
            acc.push('d')
        }
        'ǳ' | 'ǆ' => acc.push_str("dz"),
        'e' | 'ⓔ' | 'ｅ' | 'è' | 'é' | 'ê' | 'ề' | 'ế' | 'ễ' | 'ể' | 'ẽ' | 'ē' | 'ḕ' | 'ḗ'
        | 'ĕ' | 'ė' | 'ë' | 'ẻ' | 'ě' | 'ȅ' | 'ȇ' | 'ẹ' | 'ệ' | 'ȩ' | 'ḝ' | 'ę' | 'ḙ' | 'ḛ'
        | 'ɇ' | 'ɛ' | 'ǝ' => acc.push('e'),
        'f' | 'ⓕ' | 'ｆ' | 'ḟ' | 'ƒ' | 'ꝼ' => acc.push('f'),
        'g' | 'ⓖ' | 'ｇ' | 'ǵ' | 'ĝ' | 'ḡ' | 'ğ' | 'ġ' | 'ǧ' | 'ģ' | 'ǥ' | 'ɠ' | 'ꞡ' | 'ᵹ'
        | 'ꝿ' => acc.push('g'),
        'h' | 'ⓗ' | 'ｈ' | 'ĥ' | 'ḣ' | 'ḧ' | 'ȟ' | 'ḥ' | 'ḩ' | 'ḫ' | 'ẖ' | 'ħ' | 'ⱨ' | 'ⱶ'
        | 'ɥ' => acc.push('h'),
        'ƕ' => acc.push_str("hv"),
        'i' | 'ⓘ' | 'ｉ' | 'ì' | 'í' | 'î' | 'ĩ' | 'ī' | 'ĭ' | 'ï' | 'ḯ' | 'ỉ' | 'ǐ' | 'ȉ'
        | 'ȋ' | 'ị' | 'į' | 'ḭ' | 'ɨ' | 'ı' => acc.push('i'),
        'j' | 'ⓙ' | 'ｊ' | 'ĵ' | 'ǰ' | 'ɉ' => acc.push('j'),
        'k' | 'ⓚ' | 'ｋ' | 'ḱ' | 'ǩ' | 'ḳ' | 'ķ' | 'ḵ' | 'ƙ' | 'ⱪ' | 'ꝁ' | 'ꝃ' | 'ꝅ' | 'ꞣ' => {
            acc.push('k')
        }
        'l' | 'ⓛ' | 'ｌ' | 'ŀ' | 'ĺ' | 'ľ' | 'ḷ' | 'ḹ' | 'ļ' | 'ḽ' | 'ḻ' | 'ſ' | 'ł' | 'ƚ'
        | 'ɫ' | 'ⱡ' | 'ꝉ' | 'ꞁ' | 'ꝇ' => acc.push('l'),
        'ǉ' => acc.push_str("lj"),
        'm' | 'ⓜ' | 'ｍ' | 'ḿ' | 'ṁ' | 'ṃ' | 'ɱ' | 'ɯ' => acc.push('m'),
        'n' | 'ⓝ' | 'ｎ' | 'ǹ' | 'ń' | 'ñ' | 'ṅ' | 'ň' | 'ṇ' | 'ņ' | 'ṋ' | 'ṉ' | 'ƞ' | 'ɲ'
        | 'ŉ' | 'ꞑ' | 'ꞥ' => acc.push('n'),
        'ǌ' => acc.push_str("nj"),
        'o' | 'ⓞ' | 'ｏ' | 'ò' | 'ó' | 'ô' | 'ồ' | 'ố' | 'ỗ' | 'ổ' | 'õ' | 'ṍ' | 'ȭ' | 'ṏ'
        | 'ō' | 'ṑ' | 'ṓ' | 'ŏ' | 'ȯ' | 'ȱ' | 'ö' | 'ȫ' | 'ỏ' | 'ő' | 'ǒ' | 'ȍ' | 'ȏ' | 'ơ'
        | 'ờ' | 'ớ' | 'ỡ' | 'ở' | 'ợ' | 'ọ' | 'ộ' | 'ǫ' | 'ǭ' | 'ø' | 'ǿ' | 'ɔ' | 'ꝋ' | 'ꝍ'
        | 'ɵ' => acc.push('o'),
        'ƣ' => acc.push_str("oi"),
        'ȣ' => acc.push_str("ou"),
        'ꝏ' => acc.push_str("oo"),
        'p' | 'ⓟ' | 'ｐ' | 'ṕ' | 'ṗ' | 'ƥ' | 'ᵽ' | 'ꝑ' | 'ꝓ' | 'ꝕ' => acc.push('p'),
        'q' | 'ⓠ' | 'ｑ' | 'ɋ' | 'ꝗ' | 'ꝙ' => acc.push('q'),
        'r' | 'ⓡ' | 'ｒ' | 'ŕ' | 'ṙ' | 'ř' | 'ȑ' | 'ȓ' | 'ṛ' | 'ṝ' | 'ŗ' | 'ṟ' | 'ɍ' | 'ɽ'
        | 'ꝛ' | 'ꞧ' | 'ꞃ' => acc.push('r'),
        's' | 'ⓢ' | 'ｓ' | 'ß' | 'ś' | 'ṥ' | 'ŝ' | 'ṡ' | 'š' | 'ṧ' | 'ṣ' | 'ṩ' | 'ș' | 'ş'
        | 'ȿ' | 'ꞩ' | 'ꞅ' | 'ẛ' => acc.push('s'),
        't' | 'ⓣ' | 'ｔ' | 'ṫ' | 'ẗ' | 'ť' | 'ṭ' | 'ț' | 'ţ' | 'ṱ' | 'ṯ' | 'ŧ' | 'ƭ' | 'ʈ'
        | 'ⱦ' | 'ꞇ' => acc.push('t'),
        'ꜩ' => acc.push_str("tz"),
        'u' | 'ⓤ' | 'ｕ' | 'ù' | 'ú' | 'û' | 'ũ' | 'ṹ' | 'ū' | 'ṻ' | 'ŭ' | 'ü' | 'ǜ' | 'ǘ'
        | 'ǖ' | 'ǚ' | 'ủ' | 'ů' | 'ű' | 'ǔ' | 'ȕ' | 'ȗ' | 'ư' | 'ừ' | 'ứ' | 'ữ' | 'ử' | 'ự'
        | 'ụ' | 'ṳ' | 'ų' | 'ṷ' | 'ṵ' | 'ʉ' => acc.push('u'),
        'v' | 'ⓥ' | 'ｖ' | 'ṽ' | 'ṿ' | 'ʋ' | 'ꝟ' | 'ʌ' => acc.push('v'),
        'ꝡ' => acc.push_str("vy"),
        'w' | 'ⓦ' | 'ｗ' | 'ẁ' | 'ẃ' | 'ŵ' | 'ẇ' | 'ẅ' | 'ẘ' | 'ẉ' | 'ⱳ' => {
            acc.push('w')
        }
        'x' | 'ⓧ' | 'ｘ' | 'ẋ' | 'ẍ' => acc.push('x'),
        'y' | 'ⓨ' | 'ｙ' | 'ỳ' | 'ý' | 'ŷ' | 'ỹ' | 'ȳ' | 'ẏ' | 'ÿ' | 'ỷ' | 'ẙ' | 'ỵ' | 'ƴ'
        | 'ɏ' | 'ỿ' => acc.push('y'),
        'z' | 'ⓩ' | 'ｚ' | 'ź' | 'ẑ' | 'ż' | 'ž' | 'ẓ' | 'ẕ' | 'ƶ' | 'ȥ' | 'ɀ' | 'ⱬ' | 'ꝣ' => {
            acc.push('z')
        }
        '\u{0300}'..='\u{036F}' | '\u{1AB0}'..='\u{1AFF}' | '\u{1DC0}'..='\u{1DFF}' => {}
        _ => acc.push(current),
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_log_dir_is_non_empty_and_has_correct_suffix() {
        let dir = app_log_dir();
        assert!(!dir.as_os_str().is_empty(), "log dir must not be empty");
        let name = dir
            .file_name()
            .and_then(|n| n.to_str())
            .expect("log dir must have a file name component");
        // In test builds cfg(debug_assertions) is active, so the suffix is kimun_debug.
        // The path must also be absolute — fallback chain ends at temp_dir(), never a relative dot.
        assert!(dir.is_absolute(), "log dir must be an absolute path");
        assert_eq!(name, "kimun_debug");
    }

    #[test]
    fn test_uppercase() {
        assert_eq!(remove_diacritics("TÅRÖÄÆØ"), String::from("TAROAAO"))
    }
    #[test]
    fn test_lowercase() {
        assert_eq!(remove_diacritics("čďêƒíó"), String::from("cdefio"))
    }
    #[test]
    fn test_real_diacritics() {
        // this is not a traditional é, but a combination of e and \u{300}
        assert_eq!(remove_diacritics("é"), String::from("e"));
        assert_eq!(remove_diacritics("e\u{300}"), String::from("e"));
    }
}
