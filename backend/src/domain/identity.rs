//! Email addresses and phone numbers as proof of who someone is (DESIGN.md §8).

/// A normalized identifier: what is stored, compared and sent to.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Identifier {
    /// Lower-case, without surrounding space.
    Email(String),
    /// E.164: a plus sign, then country code and number, digits only.
    Phone(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("not a usable email address or phone number")]
pub struct InvalidIdentifier;

impl Identifier {
    /// Reads what someone typed. Anything containing `@` is treated as an
    /// email address; anything else must be a phone number. Formatting
    /// characters in phone numbers (spaces, dashes, dots, brackets) are
    /// dropped. A number in international form, `+` and its country code, is
    /// taken as given; one without, as people in the US write theirs, is a
    /// `+1` number: ten digits, or eleven starting with 1, as in
    /// `(856) 548-8780` or `1-856-548-8780`. Either way the result is E.164,
    /// so the same number is the same identifier however it was typed.
    ///
    /// This checks shape only. Whether the address or number exists is proved
    /// by the one-time code.
    pub fn parse(input: &str) -> Result<Self, InvalidIdentifier> {
        let input = input.trim();
        if input.contains('@') {
            Self::email(input)
        } else {
            Self::phone(input)
        }
    }

    fn email(input: &str) -> Result<Self, InvalidIdentifier> {
        let email = input.to_lowercase();
        let (local, domain) = email.rsplit_once('@').ok_or(InvalidIdentifier)?;

        // ASCII only, for now. Outside it, lowercasing is not one agreed
        // function: this code and the database can disagree about a letter,
        // and then the same address is two addresses.
        let valid = email.is_ascii()
            && email.len() <= 254
            && !local.is_empty()
            && !local.contains('@')
            && domain.contains('.')
            && domain.split('.').all(|label| !label.is_empty())
            && !email.chars().any(|c| c.is_whitespace() || c.is_control());

        valid.then_some(Self::Email(email)).ok_or(InvalidIdentifier)
    }

    fn phone(input: &str) -> Result<Self, InvalidIdentifier> {
        let (international, rest) = match input.strip_prefix('+') {
            Some(rest) => (true, rest),
            None => (false, input),
        };
        if !rest
            .chars()
            .all(|c| c.is_ascii_digit() || matches!(c, ' ' | '-' | '(' | ')' | '.'))
        {
            return Err(InvalidIdentifier);
        }
        let digits: String = rest.chars().filter(char::is_ascii_digit).collect();

        if international {
            // A +1 number is the country code and ten digits; any other is
            // left to the country's own plan, within E.164's length.
            let valid = if let Some(national) = digits.strip_prefix('1') {
                national.len() == 10
            } else {
                (7..=15).contains(&digits.len()) && !digits.starts_with('0')
            };
            return valid
                .then(|| Self::Phone(format!("+{digits}")))
                .ok_or(InvalidIdentifier);
        }

        // Without a country code, a number in the North American plan: ten
        // digits, after a leading 1 if there is one, whose area code and
        // exchange each begin with 2 to 9.
        let national = match digits.len() {
            10 => digits.as_str(),
            11 if digits.starts_with('1') => &digits[1..],
            _ => return Err(InvalidIdentifier),
        };
        let bytes = national.as_bytes();
        let valid = matches!(bytes[0], b'2'..=b'9') && matches!(bytes[3], b'2'..=b'9');
        valid
            .then(|| Self::Phone(format!("+1{national}")))
            .ok_or(InvalidIdentifier)
    }

    pub fn as_str(&self) -> &str {
        match self {
            Identifier::Email(value) | Identifier::Phone(value) => value,
        }
    }

    /// For showing someone which identifier was used without revealing it in
    /// full: `a•••@example.com`, `+1•••••••42`.
    pub fn masked(&self) -> String {
        match self {
            Identifier::Email(email) => {
                let (local, domain) = email.rsplit_once('@').expect("validated at parse");
                let first = local.chars().next().expect("validated at parse");
                format!("{first}•••@{domain}")
            }
            Identifier::Phone(phone) => {
                let digits = &phone[1..];
                let hidden = "•".repeat(digits.len() - 3);
                format!("+{}{}{}", &digits[..1], hidden, &digits[digits.len() - 2..])
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn email_addresses_are_trimmed_and_lowercased() {
        assert_eq!(
            Identifier::parse("  Ana.Ruiz@Example.COM "),
            Ok(Identifier::Email("ana.ruiz@example.com".into()))
        );
    }

    #[test]
    fn malformed_email_addresses_are_refused() {
        for bad in [
            "@example.com",
            "ana@",
            "ana@example",
            "ana@.com",
            "ana@example..com",
            "ana@@example.com",
            "ana ruiz@example.com",
            "ana@exam ple.com",
            "aña@example.com",
            "ana@exämple.com",
            "\u{38d}na@example.com",
        ] {
            assert_eq!(Identifier::parse(bad), Err(InvalidIdentifier), "{bad}");
        }
    }

    #[test]
    fn phone_numbers_are_reduced_to_e164() {
        for input in ["+12025550142", "+1 (202) 555-0142", " +1.202.555.0142 "] {
            assert_eq!(
                Identifier::parse(input),
                Ok(Identifier::Phone("+12025550142".into())),
                "{input}"
            );
        }
    }

    #[test]
    fn us_numbers_need_no_country_code() {
        for input in [
            "8565488780",
            "856-548-8780",
            "(856) 548-8780",
            "856.548.8780",
            "856 548 8780",
            "1 856 548 8780",
            "1-856-548-8780",
            "18565488780",
            "+1 856 548 8780",
            "+18565488780",
            " (856)548-8780 ",
        ] {
            assert_eq!(
                Identifier::parse(input),
                Ok(Identifier::Phone("+18565488780".into())),
                "{input}"
            );
        }
    }

    #[test]
    fn other_country_codes_are_kept_for_the_service_to_refuse() {
        // Parsed as given; `AuthRules::check_taken` refuses them as not served.
        assert_eq!(
            Identifier::parse("+44 20 7946 0958"),
            Ok(Identifier::Phone("+442079460958".into()))
        );
    }

    #[test]
    fn phone_numbers_need_a_sane_length_and_shape() {
        for bad in [
            // Too few or too many digits without a country code.
            "202555014",
            "20255501420",
            "2 202 555 0142",
            // An area code or exchange starting with 0 or 1.
            "0125550142",
            "1125550142",
            "2021550142",
            "2020550142",
            "1 202 155 0142",
            // A +1 number is ten digits after the country code.
            "+1202555014",
            "+120255501420",
            "+0123456789",
            "+123456",
            "+1234567890123456",
            "+1 202 555 01x2",
            "202 555 01x2",
            "++12025550142",
            "202+5550142",
            "",
        ] {
            assert_eq!(Identifier::parse(bad), Err(InvalidIdentifier), "{bad}");
        }
    }

    #[test]
    fn masking_hides_most_of_the_identifier() {
        assert_eq!(
            Identifier::parse("ana.ruiz@example.com").unwrap().masked(),
            "a•••@example.com"
        );
        assert_eq!(
            Identifier::parse("+12025550142").unwrap().masked(),
            "+1••••••••42"
        );
    }
}
