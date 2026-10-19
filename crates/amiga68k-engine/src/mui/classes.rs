//! The MUI class tree the engine implements: one [`Kind`] per public class, its superclass, and the creation rule. Facts: the
//! `_MUI_<Class>_desc` descriptors of Zune (AROS workbench/libs/muimaster/classes/*.c: the class name and its super's
//! name); a class not in the table is a named refusal at `MUI_NewObjectA`, never a stand-in.
//!
//! The tree decides three things and nothing else: which attributes an object accepts (its own, then every ancestor's),
//! which children it may adopt, and the `IClass` chain a guest subclass sees. What an object IS on screen is
//! [`super::roles`].
use alloc::format;
use alloc::string::String;

#[derive(Clone, Copy, PartialEq, Eq, Debug, PartialOrd, Ord)]
pub enum Kind {
    Notify, Family, Application, Window, Menustrip, Menu, Menuitem,
    Area, Rectangle, Balance, Image, Group, Virtgroup, Scrollgroup, Register, Text, String, Cycle, Radio,
    Numeric, Slider, Gauge, Prop, Scrollbar, List, Listview, Popstring, Popasl, Popobject,
}

/// `(public class name, its superclass, kind, abstract)`. Supers come first (the IClass table is built in this order).
pub struct Class { pub name: &'static str, pub sup: Option<Kind>, pub kind: Kind, pub abstract_: bool }
const fn c(name: &'static str, sup: Option<Kind>, kind: Kind, abstract_: bool) -> Class { Class { name, sup, kind, abstract_ } }

pub const CLASSES: &[Class] = &[
    c("Notify.mui", None, Kind::Notify, false),
    c("Family.mui", Some(Kind::Notify), Kind::Family, true),
    c("Application.mui", Some(Kind::Notify), Kind::Application, false),
    c("Window.mui", Some(Kind::Notify), Kind::Window, false),
    c("Menustrip.mui", Some(Kind::Family), Kind::Menustrip, false),
    c("Menu.mui", Some(Kind::Family), Kind::Menu, false),
    c("Menuitem.mui", Some(Kind::Family), Kind::Menuitem, false),
    c("Area.mui", Some(Kind::Notify), Kind::Area, false),
    c("Rectangle.mui", Some(Kind::Area), Kind::Rectangle, false),
    c("Balance.mui", Some(Kind::Area), Kind::Balance, false),
    c("Image.mui", Some(Kind::Area), Kind::Image, false),
    c("Group.mui", Some(Kind::Area), Kind::Group, false),
    c("Virtgroup.mui", Some(Kind::Group), Kind::Virtgroup, false),
    c("Scrollgroup.mui", Some(Kind::Group), Kind::Scrollgroup, false),
    c("Register.mui", Some(Kind::Group), Kind::Register, false),
    c("Text.mui", Some(Kind::Area), Kind::Text, false),
    c("String.mui", Some(Kind::Area), Kind::String, false),
    c("Cycle.mui", Some(Kind::Group), Kind::Cycle, false),
    c("Radio.mui", Some(Kind::Group), Kind::Radio, false),
    c("Numeric.mui", Some(Kind::Area), Kind::Numeric, true),
    c("Slider.mui", Some(Kind::Numeric), Kind::Slider, false),
    c("Gauge.mui", Some(Kind::Area), Kind::Gauge, false),
    c("Prop.mui", Some(Kind::Area), Kind::Prop, false),
    c("Scrollbar.mui", Some(Kind::Group), Kind::Scrollbar, false),
    c("List.mui", Some(Kind::Group), Kind::List, false),
    c("Listview.mui", Some(Kind::Group), Kind::Listview, false),
    c("Popstring.mui", Some(Kind::Group), Kind::Popstring, true),
    c("Popasl.mui", Some(Kind::Popstring), Kind::Popasl, false),
    c("Popobject.mui", Some(Kind::Popstring), Kind::Popobject, false),
];

impl Kind {
    pub fn class(self) -> &'static Class { CLASSES.iter().find(|c| c.kind == self).expect("every Kind has a class row") }
    pub fn name(self) -> &'static str { self.class().name }
    pub fn sup(self) -> Option<Kind> { self.class().sup }
    /// Is `self` `anc` or one of its descendants?
    pub fn is_a(self, anc: Kind) -> bool {
        let mut k = Some(self);
        while let Some(x) = k { if x == anc { return true; } k = x.sup(); }
        false
    }
    /// The Area-derived classes: the ones with a place in a window's layout.
    pub fn is_area(self) -> bool { self.is_a(Kind::Area) }
    /// Groups lay out children (Cycle, Radio, Scrollbar, List... are Group subclasses in Zune but own their children).
    pub fn is_container(self) -> bool { matches!(self, Kind::Group | Kind::Virtgroup | Kind::Scrollgroup | Kind::Register) }
}

/// Is `name` one of the classes this engine implements (abstract ones included)? Anything else may be a disk class.
pub fn is_builtin(name: &str) -> bool { CLASSES.iter().any(|c| c.name == name) }

/// The kind of public class `name`, or the named refusal (an unknown class, or an abstract one).
pub fn by_name(name: &str) -> Result<Kind, String> {
    match CLASSES.iter().find(|c| c.name == name) {
        None => Err(format!("MUI class \"{name}\" is not implemented")),
        Some(c) if c.abstract_ => Err(format!("MUI class \"{name}\" is abstract (it has no instances of its own)")),
        Some(c) => Ok(c.kind),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn supers_come_first_and_every_kind_has_one_class() {
        for (i, c) in CLASSES.iter().enumerate() {
            if let Some(s) = c.sup { assert!(CLASSES[..i].iter().any(|p| p.kind == s), "{} before its super", c.name); }
            assert_eq!(CLASSES.iter().filter(|p| p.kind == c.kind).count(), 1);
        }
        assert!(Kind::Slider.is_a(Kind::Area) && Kind::Popasl.is_a(Kind::Group) && !Kind::Window.is_a(Kind::Area));
    }
    #[test]
    fn unknown_and_abstract_classes_are_named_refusals() {
        assert_eq!(by_name("Clock.mcc").unwrap_err(), "MUI class \"Clock.mcc\" is not implemented");
        assert!(by_name("Numeric.mui").unwrap_err().contains("abstract"));
        assert_eq!(by_name("Cycle.mui"), Ok(Kind::Cycle));
    }
}
