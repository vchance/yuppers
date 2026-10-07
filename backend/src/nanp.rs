//! Where a `+1` number is. The North American Numbering Plan is one country
//! code shared by the United States, Canada and some twenty Caribbean and
//! Pacific countries and territories, so the country code alone admits
//! numbers in Jamaica (`+1876`) or the Dominican Republic (`+1809`), which
//! cost more to text and are where SMS pumping sends its traffic. The area
//! code says which.
//!
//! **How a number is placed.** Canada's area codes, and those of every other
//! country and territory in the plan, are listed below from NANPA's
//! assignments, as are the codes in use that are not places at all
//! (toll-free, personal and other service codes). Every other area code is
//! the United States'. The lists run that way round because new US
//! area codes open every year as overlays, and a list of the US's own would
//! refuse real numbers from the day one opens until the list caught up,
//! while a new code for another country is rare. An unassigned or reserved
//! code is placed in the US and harms nothing: nobody has a number there. The SMS
//! provider's geographic permissions, US only, are the second check
//! (docs/deploy-render.md).

/// A part of the plan the service may text (`SMS_ALLOWED_REGIONS`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Region {
    /// The fifty states and the District of Columbia.
    Us,
    Canada,
}

impl Region {
    /// The setting's name for it: `US`, `CA`.
    pub fn parse(name: &str) -> Option<Self> {
        match name.trim().to_ascii_uppercase().as_str() {
            "US" => Some(Region::Us),
            "CA" => Some(Region::Canada),
            _ => None,
        }
    }
}

/// Canada's geographic area codes, and its non-geographic 600 and 622.
const CANADA: &[u16] = &[
    204, 226, 236, 249, 250, 257, 263, 289, 306, 343, 354, 365, 367, 368, 382, 403, 416, 418, 428,
    431, 437, 438, 450, 468, 474, 506, 514, 519, 548, 579, 581, 584, 587, 600, 604, 613, 622, 639,
    647, 672, 683, 705, 709, 742, 753, 778, 780, 782, 807, 819, 825, 867, 873, 879, 902, 905, 942,
];

/// Every other country and territory in the plan: the Caribbean's, and the
/// US territories' (Puerto Rico 787 and 939, the US Virgin Islands 340,
/// Guam 671, the Northern Mariana Islands 670, American Samoa 684), which
/// the providers' permissions list as places of their own.
const ELSEWHERE: &[u16] = &[
    242, 246, 264, 268, 284, 340, 345, 441, 473, 649, 658, 664, 670, 671, 684, 721, 758, 767, 784,
    787, 809, 829, 849, 868, 869, 876, 939,
];

/// Area codes in use that are not places: toll-free, personal
/// communications (5XX), the N00 and N11 service codes, and 456 (inbound
/// international).
fn not_a_place(code: u16) -> bool {
    const TOLL_FREE: &[u16] = &[
        800, 822, 833, 844, 855, 866, 877, 880, 881, 882, 883, 884, 885, 886, 887, 888, 889,
    ];
    const PERSONAL: &[u16] = &[
        500, 521, 522, 523, 524, 525, 526, 527, 528, 529, 532, 533, 535, 538, 542, 543, 544, 545,
        546, 547, 549, 550, 552, 553, 554, 556, 566, 577, 588,
    ];
    let (middle, last) = (code / 10 % 10, code % 10);
    (middle == 0 && last == 0)
        || (middle == 1 && last == 1)
        || code == 456
        || TOLL_FREE.contains(&code)
        || PERSONAL.contains(&code)
}

/// The region of a `+1` number's ten digits (area code first), or `None`
/// for one elsewhere in the plan, or not a place, or not ten digits.
pub fn region(national: &str) -> Option<Region> {
    if national.len() != 10 || !national.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let code: u16 = national[..3].parse().ok()?;
    // An area code begins with 2 to 9.
    if code < 200 || ELSEWHERE.contains(&code) {
        return None;
    }
    if CANADA.contains(&code) {
        return Some(Region::Canada);
    }
    if not_a_place(code) {
        return None;
    }
    Some(Region::Us)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_are_placed_by_their_area_code() {
        for us in [
            "2025550142",
            "2125550100",
            "4155550100",
            "3505550100",
            "9455550100",
        ] {
            assert_eq!(region(us), Some(Region::Us), "{us}");
        }
        for canada in ["4165550100", "6045550100", "5145550100", "9025550100"] {
            assert_eq!(region(canada), Some(Region::Canada), "{canada}");
        }
        // Jamaica, the Dominican Republic, Trinidad, Puerto Rico, Guam.
        for elsewhere in [
            "8765550100",
            "8095550100",
            "8685550100",
            "7875550100",
            "6715550100",
        ] {
            assert_eq!(region(elsewhere), None, "{elsewhere}");
        }
        // Toll-free, personal and service codes, and nonsense.
        for nowhere in [
            "8005550100",
            "8885550100",
            "5005550100",
            "5335550100",
            "9005550100",
            "4115550100",
            "4565550100",
            "1235550100",
            "0125550100",
            "202555010",
            "20255501000",
        ] {
            assert_eq!(region(nowhere), None, "{nowhere}");
        }
    }

    #[test]
    fn the_lists_do_not_overlap() {
        for code in CANADA {
            assert!(!ELSEWHERE.contains(code), "{code}");
        }
        for code in CANADA.iter().chain(ELSEWHERE) {
            assert!((200..1000).contains(code), "{code}");
        }
    }

    #[test]
    fn regions_are_named_as_the_setting_names_them() {
        assert_eq!(Region::parse("US"), Some(Region::Us));
        assert_eq!(Region::parse(" ca "), Some(Region::Canada));
        assert_eq!(Region::parse("MX"), None);
        assert_eq!(Region::parse(""), None);
    }
}
