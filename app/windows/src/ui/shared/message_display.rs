//! Sender names and system-local date formatting for lists and readers.
use crate::platform::strings::wide;
use chrono::{DateTime, Datelike, Local, TimeZone, Timelike};
use mailparse::MailAddr;
use std::ptr::null;
use windows_sys::Win32::{Foundation::SYSTEMTIME, Globalization::*};

pub(crate) fn sender(value: &str) -> String {
    let Ok(addresses) = mailparse::addrparse(value) else {
        return value.trim().to_owned();
    };
    addresses
        .iter()
        .map(|address| match address {
            MailAddr::Single(single) => single
                .display_name
                .as_deref()
                .filter(|name| !name.trim().is_empty())
                .unwrap_or(&single.addr)
                .trim()
                .to_owned(),
            MailAddr::Group(group) => group.group_name.clone(),
        })
        .collect::<Vec<_>>()
        .join(", ")
}
pub(crate) fn received(value: &str) -> String {
    received_at(value, Local::now())
}
fn received_at<T: TimeZone>(value: &str, now: DateTime<T>) -> String {
    let Ok(date) = DateTime::parse_from_rfc2822(value) else {
        return value.to_owned();
    };
    let date = date.with_timezone(&now.timezone());
    let days = now
        .date_naive()
        .signed_duration_since(date.date_naive())
        .num_days();
    let time = SYSTEMTIME {
        wYear: date.year() as u16,
        wMonth: date.month() as u16,
        wDay: date.day() as u16,
        wDayOfWeek: date.weekday().num_days_from_sunday() as u16,
        wHour: date.hour() as u16,
        wMinute: date.minute() as u16,
        wSecond: date.second() as u16,
        wMilliseconds: 0,
    };
    let show_time = date.year() == now.year() && matches!(days, 0 | 1);
    let mut buffer = [0u16; 128];
    let mut pattern = [0u16; 128];
    let pattern_len =
        unsafe { GetLocaleInfoEx(null(), LOCALE_SSHORTDATE, pattern.as_mut_ptr(), 128) };
    let pattern = if pattern_len > 1 {
        String::from_utf16_lossy(&pattern[..pattern_len as usize - 1])
    } else {
        "M/d/yyyy".to_owned()
    };
    let pattern = wide(&if date.year() == now.year() {
        without_year(&pattern)
    } else {
        pattern
    });
    let count = unsafe {
        if show_time {
            GetTimeFormatEx(
                null(),
                TIME_NOSECONDS,
                &time,
                null(),
                buffer.as_mut_ptr(),
                128,
            )
        } else {
            GetDateFormatEx(
                null(),
                0,
                &time,
                pattern.as_ptr(),
                buffer.as_mut_ptr(),
                128,
                null(),
            )
        }
    };
    if count <= 1 {
        return value.to_owned();
    }
    let text = String::from_utf16_lossy(&buffer[..count as usize - 1]);
    if show_time && days == 1 {
        format!("Yesterday {text}")
    } else {
        text
    }
}

// Preserve Windows month/day order and quoted literals when omitting the year.
fn without_year(pattern: &str) -> String {
    let mut parts: Vec<(bool, bool, String)> = Vec::new();
    let mut chars = pattern.chars().peekable();
    while let Some(c) = chars.next() {
        let field = matches!(c, 'y' | 'M' | 'd' | 'g');
        let mut text = c.to_string();
        if c == '\'' {
            for next in chars.by_ref() {
                text.push(next);
                if next == '\'' {
                    break;
                }
            }
        } else if field {
            while chars.peek() == Some(&c) {
                text.push(chars.next().unwrap());
            }
        }
        if !field && parts.last().is_some_and(|p| !p.1) {
            parts.last_mut().unwrap().2.push_str(&text);
        } else {
            parts.push((c == 'y' && field, field, text));
        }
    }
    while let Some(i) = parts.iter().position(|p| p.0) {
        parts.remove(i);
        if i < parts.len() && !parts[i].1 {
            parts.remove(i);
        } else if i > 0 && !parts[i - 1].1 {
            parts.remove(i - 1);
        }
    }
    let result: String = parts.into_iter().map(|p| p.2).collect();
    if result.trim().is_empty() {
        "M/d".to_owned()
    } else {
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sender_names_hide_addresses_but_address_only_headers_remain_identifiable() {
        assert_eq!(sender("X <noreply@example.test>"), "X");
        assert_eq!(
            sender("\"Discover by Capital One\" <notice@example.test>"),
            "Discover by Capital One"
        );
        assert_eq!(sender("\"姓, 名\" <reader@example.test>"), "姓, 名");
        assert_eq!(sender("reader@example.test"), "reader@example.test");
    }
    #[test]
    fn system_patterns_omit_year_without_changing_order() {
        for (pattern, expected) in [
            ("M/d/yyyy", "M/d"),
            ("dd.MM.yyyy", "dd.MM"),
            ("yyyy-MM-dd", "MM-dd"),
            ("dd MMM yyyy", "dd MMM"),
        ] {
            assert_eq!(without_year(pattern), expected);
        }
        let now = DateTime::parse_from_rfc3339("2026-10-10T12:00:00+08:00").unwrap();
        assert!(received_at("Fri, 09 Oct 2026 15:37:00 +0800", now).starts_with("Yesterday "));
        assert_eq!(
            received_at("Fri, 09 Oct 2026 20:17:22 +0000", now),
            received_at("Sat, 10 Oct 2026 04:17:22 +0800", now)
        );
        let january = DateTime::parse_from_rfc3339("2027-01-01T00:01:00+08:00").unwrap();
        assert!(received_at("Thu, 31 Dec 2026 23:59:00 +0800", january).contains("2026"));
        assert_eq!(received_at("invalid date", now), "invalid date");
    }
}
