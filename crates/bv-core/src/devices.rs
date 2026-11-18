//! Device list model (FR-040 to FR-047, FR-066, FR-087).
//!
//! Combines what the user configured (the ordered list of known devices, with custom names
//! and visibility, stored in the settings) with what Windows reports right now, and
//! produces the list the flyout and Home Assistant show. The same function serves both,
//! so they can never disagree (FR-112).

use crate::audio::DeviceInfo;

/// A device the application has already seen, as remembered in the settings (FR-042).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KnownDevice {
    /// Stable Windows identifier (FR-066).
    pub id: String,
    /// The name Windows gave the device when it was last seen, so that a disconnected
    /// device can still be listed and named (FR-089). Empty if never recorded.
    pub windows_name: String,
    /// Name chosen by the user. `None` or blank = the Windows name (FR-087).
    pub custom_name: Option<String>,
    /// Hidden by the user (FR-085).
    pub hidden: bool,
    /// Icon chosen by the user: a catalogue identifier (FR-088). `None` = automatic
    /// (FR-067). An identifier the catalogue does not know is treated as automatic.
    pub icon: Option<String>,
}

impl KnownDevice {
    /// A newly seen device: visible, with its Windows name (FR-044).
    pub fn new(id: &str) -> Self {
        Self { id: id.to_owned(), ..Self::default() }
    }

    /// A newly seen device, remembering the name Windows gives it.
    pub fn seen(info: &DeviceInfo) -> Self {
        Self { id: info.id.clone(), windows_name: info.windows_name.clone(), ..Self::default() }
    }

    /// The custom name if the user set a non-blank one.
    pub fn custom_name(&self) -> Option<&str> {
        self.custom_name.as_deref().map(str::trim).filter(|name| !name.is_empty())
    }
}

/// One entry of the device list shown to the user.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VisibleDevice<'a> {
    /// What Windows reports about the device.
    pub info: &'a DeviceInfo,
    /// Custom name, or the Windows name, numbered when equal to another one.
    pub display_name: String,
    /// This is the current device of its flow (FR-041).
    pub current: bool,
    /// Hidden by the user but shown because it is the current device (FR-045).
    pub temporary: bool,
}

/// Appends to `known`, at the end and visible, every available device never seen before
/// (FR-044). Unavailable devices are not learnt: Windows keeps many stale endpoints, and a
/// device only enters the list once it has really been usable.
///
/// Returns `true` if `known` changed (the settings then need saving).
///
/// Devices learnt in the same pass are appended in natural alphabetical order of their
/// Windows name ("2 Speakers" before "10 Headset", case ignored), so a first start
/// shows a predictable order rather than the arbitrary Windows one. The Windows name of
/// every known device Windows still reports is kept up to date (FR-089).
pub fn learn(known: &mut Vec<KnownDevice>, live: &[DeviceInfo]) -> bool {
    let mut renamed = false;
    for entry in known.iter_mut() {
        let reported = live.iter().find(|d| d.id == entry.id).map(|d| d.windows_name.as_str());
        if let Some(name) = reported.filter(|name| !name.is_empty() && *name != entry.windows_name)
        {
            entry.windows_name = name.to_owned();
            renamed = true;
        }
    }
    let mut new: Vec<&DeviceInfo> = Vec::new();
    for device in live {
        let seen = known.iter().any(|k| k.id == device.id) || new.iter().any(|d| d.id == device.id);
        if device.state.is_available() && !seen {
            new.push(device);
        }
    }
    new.sort_by(|a, b| natural_cmp(&a.windows_name, &b.windows_name).then_with(|| a.id.cmp(&b.id)));
    let changed = renamed || !new.is_empty();
    known.extend(new.into_iter().map(KnownDevice::seen));
    changed
}

/// The displayed names of one device list, in list order: every known device
/// (first entry of an identifier only), then the devices Windows reports that are not
/// known yet. A device shows its custom name, else its Windows name (the live one, else
/// the one last recorded); names equal to an earlier one (case ignored) get " (2)",
/// " (3)"... so that every displayed name is unique (Home Assistant needs it, FR-087).
pub fn display_names(known: &[KnownDevice], live: &[DeviceInfo]) -> Vec<(String, String)> {
    let mut bases: Vec<(String, String)> = Vec::with_capacity(known.len() + live.len());
    for entry in known {
        if bases.iter().any(|(id, _)| *id == entry.id) {
            continue;
        }
        let live_name = live.iter().find(|d| d.id == entry.id).map(|d| d.windows_name.as_str());
        let base = entry.custom_name().or(live_name).unwrap_or(&entry.windows_name);
        bases.push((entry.id.clone(), base.to_owned()));
    }
    for info in live {
        if !bases.iter().any(|(id, _)| *id == info.id) {
            bases.push((info.id.clone(), info.windows_name.clone()));
        }
    }
    let lower: Vec<String> = bases.iter().map(|(_, name)| name.to_lowercase()).collect();
    let mut taken: Vec<String> = Vec::with_capacity(bases.len());
    let mut names = Vec::with_capacity(bases.len());
    for ((id, base), base_lower) in bases.into_iter().zip(&lower) {
        let name = if taken.contains(base_lower) {
            // The smallest number giving a name nobody has, nor will have as its own.
            (2..)
                .map(|n| format!("{base} ({n})"))
                .find(|candidate| {
                    let candidate = candidate.to_lowercase();
                    !taken.contains(&candidate) && !lower.contains(&candidate)
                })
                .unwrap_or(base)
        } else {
            base
        };
        taken.push(name.to_lowercase());
        names.push((id, name));
    }
    names
}

/// Natural order of two names: runs of digits compare by value, other characters by
/// their lower-case form.
pub fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let (mut a, mut b) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (a.peek().copied(), b.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let number = |it: &mut std::iter::Peekable<std::str::Chars<'_>>| {
                    let mut digits = String::new();
                    while let Some(c) = it.peek().copied().filter(char::is_ascii_digit) {
                        digits.push(c);
                        it.next();
                    }
                    digits
                };
                let (da, db) = (number(&mut a), number(&mut b));
                let (ta, tb) = (da.trim_start_matches('0'), db.trim_start_matches('0'));
                let order = ta.len().cmp(&tb.len()).then_with(|| ta.cmp(tb));
                if order != Ordering::Equal {
                    return order;
                }
            }
            (Some(x), Some(y)) => {
                let order = x.to_lowercase().cmp(y.to_lowercase());
                if order != Ordering::Equal {
                    return order;
                }
                a.next();
                b.next();
            }
        }
    }
}

/// The name to show for `info`: its entry of [`display_names`].
pub fn display_name(known: &[KnownDevice], live: &[DeviceInfo], info: &DeviceInfo) -> String {
    name_in(&display_names(known, live), info)
}

fn name_in(names: &[(String, String)], info: &DeviceInfo) -> String {
    names
        .iter()
        .find(|(id, _)| *id == info.id)
        .map_or_else(|| info.windows_name.clone(), |(_, name)| name.clone())
}

/// The device list to show, in order:
///
/// 1. known devices in their configured order, if available and not hidden (FR-042,
///    FR-043);
/// 2. available devices not known yet, in Windows order (FR-044);
/// 3. last, the current device if the user hid it (FR-045), marked as temporary.
///
/// Unavailable devices never appear, even when current (FR-043). Each device appears once,
/// even if the stored list is damaged and repeats an identifier.
pub fn visible<'a>(
    known: &'a [KnownDevice],
    live: &'a [DeviceInfo],
    current: Option<&str>,
) -> Vec<VisibleDevice<'a>> {
    let names = display_names(known, live);
    let mut shown: Vec<VisibleDevice<'a>> = Vec::with_capacity(live.len());
    let mut hidden_current: Option<VisibleDevice<'a>> = None;
    let is_listed = |shown: &[VisibleDevice<'_>], id: &str| shown.iter().any(|v| v.info.id == id);

    for (position, entry) in known.iter().enumerate() {
        // A repeated identifier (damaged settings): only its first entry counts.
        if known[..position].iter().any(|k| k.id == entry.id) {
            continue;
        }
        let Some(info) = live.iter().find(|d| d.id == entry.id) else { continue };
        if !info.state.is_available() || is_listed(&shown, &info.id) {
            continue;
        }
        let is_current = current == Some(info.id.as_str());
        let device = VisibleDevice {
            info,
            display_name: name_in(&names, info),
            current: is_current,
            temporary: entry.hidden,
        };
        if !entry.hidden {
            shown.push(device);
        } else if is_current && hidden_current.is_none() {
            hidden_current = Some(device);
        }
    }

    for info in live {
        let known_already = known.iter().any(|k| k.id == info.id);
        if info.state.is_available() && !known_already && !is_listed(&shown, &info.id) {
            shown.push(VisibleDevice {
                info,
                display_name: name_in(&names, info),
                current: current == Some(info.id.as_str()),
                temporary: false,
            });
        }
    }

    shown.extend(hidden_current);
    shown
}

/// Why a name was refused (FR-087).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RenameError {
    /// No device with this identifier in the list.
    Unknown,
    /// Another device of the list already shows this name (case ignored).
    Duplicate,
}

/// Moves the device `id` to position `to` of the list (FR-086). Returns `true` if the
/// order changed.
pub fn move_to(known: &mut Vec<KnownDevice>, id: &str, to: usize) -> bool {
    let Some(from) = known.iter().position(|k| k.id == id) else { return false };
    let to = to.min(known.len() - 1);
    if from == to {
        return false;
    }
    let entry = known.remove(from);
    known.insert(to, entry);
    true
}

/// Renames the device `id` (FR-087): `name` is trimmed; empty, or equal to the Windows
/// name, means "no custom name". A name another device of the list shows is refused.
/// Returns whether anything changed.
pub fn rename(
    known: &mut [KnownDevice],
    live: &[DeviceInfo],
    id: &str,
    name: &str,
) -> Result<bool, RenameError> {
    let names = display_names(known, live);
    let name = name.trim();
    let lower = name.to_lowercase();
    if !name.is_empty()
        && names.iter().any(|(other, shown)| other != id && shown.to_lowercase() == lower)
    {
        return Err(RenameError::Duplicate);
    }
    let live_name = live.iter().find(|d| d.id == id).map(|d| d.windows_name.clone());
    let entry = known.iter_mut().find(|k| k.id == id).ok_or(RenameError::Unknown)?;
    let windows_name = live_name.unwrap_or_else(|| entry.windows_name.clone());
    let custom = (!name.is_empty() && name != windows_name).then(|| name.to_owned());
    let changed = entry.custom_name != custom;
    entry.custom_name = custom;
    Ok(changed)
}

/// Shows or hides the device `id` in the flyout (FR-085). Returns `true` if it changed.
pub fn set_hidden(known: &mut [KnownDevice], id: &str, hidden: bool) -> bool {
    known
        .iter_mut()
        .find(|k| k.id == id)
        .is_some_and(|k| std::mem::replace(&mut k.hidden, hidden) != hidden)
}

/// Sets the icon of the device `id` (a catalogue identifier, `None` = automatic, FR-088).
/// Returns `true` if it changed.
pub fn set_icon(known: &mut [KnownDevice], id: &str, icon: Option<&str>) -> bool {
    let icon = icon.map(str::to_owned);
    known
        .iter_mut()
        .find(|k| k.id == id)
        .is_some_and(|k| std::mem::replace(&mut k.icon, icon.clone()) != icon)
}

/// Forgets the device `id` (FR-089): only a device Windows does not report as available
/// can be forgotten (an available one would be learnt again at once). Returns `true` if
/// it was removed.
pub fn forget(known: &mut Vec<KnownDevice>, live: &[DeviceInfo], id: &str) -> bool {
    let available = live.iter().any(|d| d.id == id && d.state.is_available());
    let before = known.len();
    if !available {
        known.retain(|k| k.id != id);
    }
    known.len() != before
}

#[cfg(test)]
mod edit_tests {
    use super::*;
    use crate::audio::DeviceState;

    fn live(id: &str, name: &str) -> DeviceInfo {
        DeviceInfo {
            id: id.into(),
            windows_name: name.into(),
            state: DeviceState::Active,
            ..Default::default()
        }
    }

    fn list() -> Vec<KnownDevice> {
        ["a", "b", "c"]
            .iter()
            .map(|id| KnownDevice { windows_name: format!("Windows {id}"), ..KnownDevice::new(id) })
            .collect()
    }

    fn ids(known: &[KnownDevice]) -> Vec<&str> {
        known.iter().map(|k| k.id.as_str()).collect()
    }

    #[test]
    fn devices_move_to_their_new_place() {
        let mut known = list();
        assert!(move_to(&mut known, "a", 2));
        assert_eq!(ids(&known), ["b", "c", "a"]);
        assert!(move_to(&mut known, "a", 0));
        assert_eq!(ids(&known), ["a", "b", "c"]);
        assert!(!move_to(&mut known, "b", 1), "already there");
        assert!(move_to(&mut known, "a", 99), "beyond the end: last");
        assert_eq!(ids(&known), ["b", "c", "a"]);
        assert!(!move_to(&mut known, "zz", 0));
    }

    #[test]
    fn renaming_trims_restores_and_refuses_duplicates() {
        let mut known = list();
        let live = [live("a", "Windows a"), live("b", "Windows b")];
        assert_eq!(rename(&mut known, &live, "a", "  Desk "), Ok(true));
        assert_eq!(known[0].custom_name.as_deref(), Some("Desk"));
        assert_eq!(rename(&mut known, &live, "a", "Desk"), Ok(false));
        // Another device's displayed name, in any case: refused.
        assert_eq!(rename(&mut known, &live, "b", "DESK"), Err(RenameError::Duplicate));
        assert_eq!(rename(&mut known, &live, "b", "windows c"), Err(RenameError::Duplicate));
        // Empty, or the Windows name itself: back to the Windows name.
        assert_eq!(rename(&mut known, &live, "a", ""), Ok(true));
        assert_eq!(known[0].custom_name, None);
        assert_eq!(rename(&mut known, &live, "a", "Windows a"), Ok(false));
        assert_eq!(rename(&mut known, &live, "zz", "x"), Err(RenameError::Unknown));
    }

    #[test]
    fn hiding_icons_and_forgetting() {
        let mut known = list();
        assert!(set_hidden(&mut known, "b", true));
        assert!(!set_hidden(&mut known, "b", true));
        assert!(known[1].hidden);
        assert!(set_icon(&mut known, "c", Some("soundbar")));
        assert!(!set_icon(&mut known, "c", Some("soundbar")));
        assert!(set_icon(&mut known, "c", None));
        // An available device cannot be forgotten; a disconnected one can.
        let live = [live("a", "Windows a")];
        assert!(!forget(&mut known, &live, "a"));
        assert!(forget(&mut known, &live, "c"));
        assert_eq!(ids(&known), ["a", "b"]);
    }
}

#[cfg(test)]
mod order_tests {
    use super::*;
    use crate::audio::DeviceState;

    fn device(id: &str, name: &str) -> DeviceInfo {
        DeviceInfo {
            id: id.into(),
            windows_name: name.into(),
            state: DeviceState::Active,
            ..Default::default()
        }
    }

    #[test]
    fn devices_learnt_together_are_in_natural_alphabetical_order() {
        let mut known = vec![KnownDevice::new("old")];
        let live = [
            device("a", "10 Headset"),
            device("b", "2 Speakers"),
            device("c", "casque"),
            device("d", "Barre de son"),
            device("old", "0 First"),
        ];
        assert!(learn(&mut known, &live));
        let ids: Vec<&str> = known.iter().map(|k| k.id.as_str()).collect();
        // Already known devices keep their place; new ones follow, sorted.
        assert_eq!(ids, ["old", "b", "a", "d", "c"]);
        assert!(!learn(&mut known, &live), "nothing new the second time");
    }

    #[test]
    fn natural_order_compares_numbers_by_value() {
        use std::cmp::Ordering::*;
        assert_eq!(natural_cmp("1.Casque", "2.Enceintes"), Less);
        assert_eq!(natural_cmp("9 a", "10 a"), Less);
        assert_eq!(natural_cmp("a", "B"), Less);
        assert_eq!(natural_cmp("x02", "x2"), Equal);
        assert_eq!(natural_cmp("ab", "a"), Greater);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::DeviceState;

    fn device(id: &str, state: DeviceState) -> DeviceInfo {
        DeviceInfo {
            id: id.into(),
            windows_name: format!("Windows {id}"),
            state,
            ..Default::default()
        }
    }

    fn active(id: &str) -> DeviceInfo {
        device(id, DeviceState::Active)
    }

    fn known(id: &str) -> KnownDevice {
        KnownDevice::new(id)
    }

    fn hidden(id: &str) -> KnownDevice {
        KnownDevice { hidden: true, ..KnownDevice::new(id) }
    }

    fn renamed(id: &str, name: &str) -> KnownDevice {
        KnownDevice { custom_name: Some(name.into()), ..KnownDevice::new(id) }
    }

    fn ids<'a>(list: &[VisibleDevice<'a>]) -> Vec<&'a str> {
        list.iter().map(|v| v.info.id.as_str()).collect()
    }

    #[test]
    fn configured_order_is_kept() {
        let live = [active("a"), active("b"), active("c")];
        let known = [known("c"), known("a"), known("b")];
        assert_eq!(ids(&visible(&known, &live, None)), ["c", "a", "b"]);
    }

    #[test]
    fn unavailable_devices_are_hidden_and_come_back_at_their_place() {
        let known = [known("a"), known("b"), known("c")];
        let unplugged = [active("a"), device("b", DeviceState::Unplugged), active("c")];
        assert_eq!(ids(&visible(&known, &unplugged, None)), ["a", "c"]);
        let disabled = [active("a"), device("b", DeviceState::Disabled), active("c")];
        assert_eq!(ids(&visible(&known, &disabled, None)), ["a", "c"]);
        let gone = [active("a"), active("c")];
        assert_eq!(ids(&visible(&known, &gone, None)), ["a", "c"]);
        let back = [active("c"), active("b"), active("a")];
        assert_eq!(ids(&visible(&known, &back, None)), ["a", "b", "c"]);
    }

    #[test]
    fn unavailable_current_device_is_not_shown() {
        let known = [known("a"), known("b")];
        let live = [active("a"), device("b", DeviceState::Unplugged)];
        assert_eq!(ids(&visible(&known, &live, Some("b"))), ["a"]);
    }

    #[test]
    fn new_devices_are_appended_in_windows_order() {
        let known = [known("b")];
        let live = [active("z"), active("b"), active("a")];
        let list = visible(&known, &live, None);
        assert_eq!(ids(&list), ["b", "z", "a"]);
        assert_eq!(list[1].display_name, "Windows z");
        assert!(!list[1].temporary);
    }

    #[test]
    fn hidden_devices_are_not_shown() {
        let known = [known("a"), hidden("b"), known("c")];
        let live = [active("a"), active("b"), active("c")];
        assert_eq!(ids(&visible(&known, &live, Some("a"))), ["a", "c"]);
    }

    #[test]
    fn hidden_current_device_is_shown_last_while_current() {
        let known = [hidden("b"), known("a"), known("c")];
        let live = [active("a"), active("b"), active("c"), active("new")];
        let list = visible(&known, &live, Some("b"));
        assert_eq!(ids(&list), ["a", "c", "new", "b"]);
        let last = list.last().unwrap();
        assert!(last.current && last.temporary);
        assert!(list[..3].iter().all(|v| !v.current && !v.temporary));
        // Once another device is current, it disappears again.
        assert_eq!(ids(&visible(&known, &live, Some("a"))), ["a", "c", "new"]);
    }

    #[test]
    fn current_device_is_flagged() {
        let known = [known("a"), known("b")];
        let live = [active("a"), active("b"), active("n")];
        let list = visible(&known, &live, Some("b"));
        assert_eq!(list.iter().map(|v| v.current).collect::<Vec<_>>(), [false, true, false]);
        let list = visible(&known, &live, Some("n"));
        assert_eq!(list.iter().map(|v| v.current).collect::<Vec<_>>(), [false, false, true]);
        let list = visible(&known, &live, Some("unknown id"));
        assert!(list.iter().all(|v| !v.current));
    }

    #[test]
    fn custom_names_replace_windows_names_unless_blank() {
        let known = [renamed("a", "Desk speakers"), renamed("b", "   "), renamed("c", " Headset ")];
        let live = [active("a"), active("b"), active("c")];
        let list = visible(&known, &live, None);
        let names: Vec<&str> = list.iter().map(|v| v.display_name.as_str()).collect();
        assert_eq!(names, ["Desk speakers", "Windows b", "Headset"]);
        assert_eq!(display_name(&known, &live, &live[0]), "Desk speakers");
        assert_eq!(display_name(&known, &live, &live[1]), "Windows b");
        assert_eq!(display_name(&[], &live, &live[0]), "Windows a");
    }

    fn named(id: &str, name: &str) -> DeviceInfo {
        DeviceInfo { windows_name: name.into(), ..active(id) }
    }

    fn names_of(known: &[KnownDevice], live: &[DeviceInfo]) -> Vec<String> {
        display_names(known, live).into_iter().map(|(_, name)| name).collect()
    }

    #[test]
    fn identical_names_are_numbered_in_list_order() {
        // Two identical headsets, a third one, and a device renamed like them.
        let live = [named("a", "Headset"), named("b", "Headset"), named("c", "headset")];
        let stored = [known("b"), known("a"), known("c")];
        assert_eq!(names_of(&stored, &live), ["Headset", "Headset (2)", "headset (3)"]);
        let list = visible(&stored, &live, None);
        assert_eq!(list[1].display_name, "Headset (2)");
        assert_eq!(display_name(&stored, &live, &live[0]), "Headset (2)");
        // Renaming one removes its number; the others close the gap.
        let stored = [known("b"), renamed("a", "Desk"), known("c")];
        assert_eq!(names_of(&stored, &live), ["Headset", "Desk", "headset (2)"]);
    }

    #[test]
    fn numbering_never_collides_with_a_real_name() {
        let live = [named("a", "Headset"), named("b", "Headset"), named("c", "Headset (2)")];
        let stored = [known("a"), known("b"), known("c")];
        assert_eq!(names_of(&stored, &live), ["Headset", "Headset (3)", "Headset (2)"]);
    }

    #[test]
    fn disconnected_devices_keep_their_recorded_name() {
        let stored = KnownDevice { windows_name: "Headset".into(), ..known("gone") };
        let live = [named("a", "Headset")];
        let stored = [stored, known("a")];
        // The disconnected device keeps its name and its place in the numbering.
        assert_eq!(names_of(&stored, &live), ["Headset", "Headset (2)"]);
        assert_eq!(visible(&stored, &live, None)[0].display_name, "Headset (2)");
        // A device not known yet comes after the known ones.
        assert_eq!(names_of(&[], &live), ["Headset"]);
    }

    #[test]
    fn learning_records_and_refreshes_windows_names() {
        let mut list = vec![known("a")];
        assert!(learn(&mut list, &[named("a", "Speakers"), named("b", "Headset")]));
        assert_eq!(list[0].windows_name, "Speakers");
        assert_eq!(list[1], KnownDevice { windows_name: "Headset".into(), ..known("b") });
        // Renamed in Windows: recorded; an empty name never replaces a recorded one.
        assert!(learn(&mut list, &[named("a", "Desk speakers")]));
        assert_eq!(list[0].windows_name, "Desk speakers");
        assert!(!learn(&mut list, &[named("a", "")]));
        assert_eq!(list[0].windows_name, "Desk speakers");
    }

    #[test]
    fn damaged_lists_never_show_a_device_twice() {
        let stored = [known("a"), renamed("a", "again"), hidden("b"), hidden("b")];
        let live = [active("a"), active("b"), active("a")];
        let list = visible(&stored, &live, Some("b"));
        assert_eq!(ids(&list), ["a", "b"]);
        assert_eq!(list[0].display_name, "Windows a", "the first entry wins");
        // A hidden entry followed by a visible duplicate: the first one (hidden) wins.
        let stored = [hidden("a"), known("a")];
        assert_eq!(
            ids(&visible(&stored, &live, Some("a"))),
            ["b", "a"],
            "b is new; a is the hidden current one"
        );
        assert!(visible(&stored, &live, Some("a"))[1].temporary);
        assert_eq!(ids(&visible(&stored, &live, None)), ["b"]);
    }

    #[test]
    fn no_device_gives_an_empty_list() {
        assert!(visible(&[known("a")], &[], Some("a")).is_empty());
        assert!(visible(&[], &[], None).is_empty());
    }

    #[test]
    fn only_available_unseen_devices_are_learnt() {
        let mut list = vec![known("a")];
        let live = [
            active("a"),
            active("b"),
            device("c", DeviceState::Unplugged),
            device("d", DeviceState::Disabled),
            active("e"),
        ];
        assert!(learn(&mut list, &live));
        let ids: Vec<&str> = list.iter().map(|k| k.id.as_str()).collect();
        assert_eq!(ids, ["a", "b", "e"]);
        assert!(!learn(&mut list, &live), "learning twice changes nothing");
    }

    #[test]
    fn learning_keeps_existing_settings() {
        let mut list = vec![hidden("a"), renamed("b", "Mine")];
        list.iter_mut().for_each(|k| k.windows_name = format!("Windows {}", k.id));
        let before = list.clone();
        assert!(!learn(&mut list, &[active("b"), active("a")]));
        assert_eq!(list, before);
    }
}
