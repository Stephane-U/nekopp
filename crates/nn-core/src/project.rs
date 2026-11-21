//! The three project panels.
//!
//! Notepad++ gives the user three project slots, each holding one named project,
//! each project a tree of folders and files. The tree is the same widget the
//! folder-as-workspace panel uses; what a project adds is a name, a place to
//! keep it, and the ability to hold several unrelated roots at once.
//!
//! A project is a name and a list of roots. Roots are paths, not a nested
//! structure: the panel scans each root into a `Tree`, so a project that lists
//! `/opt/a` and `/opt/b` shows both trees one after the other. Notepad++'s
//! `projects.xml` nests folders inside folders, which is a second way to say
//! "these paths, in this order" -- and the second way is the one that has to be
//! kept in sync with the disk. Paths are the truth; the nesting is presentation.
//!
//! The file format is the line-oriented one the rest of the configuration uses.
//! `projects.xml` would be closer to Notepad++'s own file, but this crate has no
//! XML writer and a project list is four fields deep at most.

use std::path::{Path, PathBuf};

/// How many project slots Notepad++ offers, and therefore how many we offer.
pub const MAX_PROJECTS: usize = 3;

/// One named project.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Project {
    /// What the panel's tab calls it.
    pub name: String,
    /// The directories and files it holds, in the order they were added.
    pub roots: Vec<PathBuf>,
}

impl Project {
    /// A project with no roots.
    pub fn new(name: impl Into<String>) -> Project {
        Project {
            name: name.into(),
            roots: Vec::new(),
        }
    }

    /// Adds a root, ignoring one that is already there.
    ///
    /// Returns whether it was added. A duplicate root would show the same tree
    /// twice and search the same files twice, so it is refused rather than
    /// deduplicated later.
    pub fn add_root(&mut self, path: impl Into<PathBuf>) -> bool {
        let path = path.into();
        if self.roots.contains(&path) {
            return false;
        }
        self.roots.push(path);
        true
    }

    /// Removes a root, returning whether it was there.
    pub fn remove_root(&mut self, path: &Path) -> bool {
        let before = self.roots.len();
        self.roots.retain(|r| r != path);
        self.roots.len() != before
    }

    /// Whether the project has nothing to show.
    pub fn is_empty(&self) -> bool {
        self.roots.is_empty()
    }
}

/// The three slots, and which one the panel is showing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Projects {
    /// The projects that exist, in slot order. Never more than `MAX_PROJECTS`.
    pub projects: Vec<Project>,
    /// The slot the panel is showing.
    pub active: usize,
}

impl Projects {
    /// No projects at all.
    pub fn new() -> Projects {
        Projects::default()
    }

    /// The project in the active slot, if there is one.
    pub fn active_project(&self) -> Option<&Project> {
        self.projects.get(self.active())
    }

    /// The active slot's index, clamped to a slot that exists.
    ///
    /// A file that names slot 3 when only one project exists would otherwise
    /// leave the panel pointing at nothing.
    pub fn active(&self) -> usize {
        self.active.min(self.projects.len().saturating_sub(1))
    }

    /// Shows slot `index`, if it exists.
    pub fn set_active(&mut self, index: usize) -> bool {
        if index >= self.projects.len() {
            return false;
        }
        self.active = index;
        true
    }

    /// Adds a project in the next free slot, returning its index.
    ///
    /// `None` when all three slots are taken. The caller decides what to tell
    /// the user; this type does not know about dialogs.
    pub fn add(&mut self, name: impl Into<String>) -> Option<usize> {
        if self.projects.len() >= MAX_PROJECTS {
            return None;
        }
        self.projects.push(Project::new(name));
        Some(self.projects.len() - 1)
    }

    /// Removes the project in slot `index`, returning whether it was there.
    pub fn remove(&mut self, index: usize) -> bool {
        if index >= self.projects.len() {
            return false;
        }
        self.projects.remove(index);
        self.active = self.active.min(self.projects.len().saturating_sub(1));
        true
    }

    /// Renames the project in slot `index`.
    pub fn rename(&mut self, index: usize, name: impl Into<String>) -> bool {
        match self.projects.get_mut(index) {
            Some(project) => {
                project.name = name.into();
                true
            }
            None => false,
        }
    }

    /// Every root of every project, deduplicated.
    ///
    /// This is what find-in-projects searches. Deduplicating matters because
    /// two projects that both list a repository would otherwise report every
    /// hit twice.
    pub fn all_roots(&self) -> Vec<PathBuf> {
        let mut roots: Vec<PathBuf> = Vec::new();
        for project in &self.projects {
            for root in &project.roots {
                if !roots.contains(root) {
                    roots.push(root.clone());
                }
            }
        }
        roots
    }

    /// The roots of the active project.
    pub fn active_roots(&self) -> &[PathBuf] {
        match self.projects.get(self.active()) {
            Some(project) => &project.roots,
            None => &[],
        }
    }

    /// Whether any project has a root to search.
    pub fn has_roots(&self) -> bool {
        self.projects.iter().any(|p| !p.roots.is_empty())
    }

    /// The file's text.
    pub fn render(&self) -> String {
        let mut out = String::new();
        out.push_str("# Neko++ projects\n");
        out.push_str(&format!("active {}\n", self.active()));
        for project in &self.projects {
            out.push_str(&format!("project {}\n", project.name));
            for root in &project.roots {
                out.push_str(&format!("root {}\n", root.display()));
            }
        }
        out
    }

    /// Reads the file's text.
    ///
    /// Unknown keywords are skipped, a `root` before any `project` is dropped,
    /// and a fourth project is ignored -- the same forgiving reading the
    /// settings file uses, so a file written by a newer build still loads.
    pub fn parse(text: &str) -> Projects {
        let mut projects = Projects::new();
        let mut active = 0;
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((key, value)) = line.split_once(' ') else {
                continue;
            };
            let value = value.trim();
            match key {
                "active" => active = value.parse().unwrap_or(0),
                "project" => {
                    if projects.projects.len() < MAX_PROJECTS {
                        projects.projects.push(Project::new(value));
                    }
                }
                "root" => {
                    if let Some(project) = projects.projects.last_mut() {
                        project.add_root(PathBuf::from(value));
                    }
                }
                _ => {}
            }
        }
        projects.active = active.min(projects.projects.len().saturating_sub(1));
        projects
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Projects {
        let mut projects = Projects::new();
        let index = projects.add("npp").unwrap();
        projects.projects[index].add_root("/opt/DEV/nn");
        projects.projects[index].add_root("/opt/DEV/nn/Cargo.toml");
        projects
    }

    #[test]
    fn a_new_project_has_no_roots() {
        let project = Project::new("x");
        assert!(project.is_empty());
        assert_eq!(project.name, "x");
    }

    #[test]
    fn a_duplicate_root_is_refused() {
        let mut project = Project::new("x");
        assert!(project.add_root("/a"));
        assert!(!project.add_root("/a"));
        assert_eq!(project.roots.len(), 1);
    }

    #[test]
    fn removing_a_root_reports_whether_it_was_there() {
        let mut project = Project::new("x");
        project.add_root("/a");
        assert!(project.remove_root(Path::new("/a")));
        assert!(!project.remove_root(Path::new("/a")));
        assert!(project.is_empty());
    }

    #[test]
    fn three_projects_fit_and_a_fourth_does_not() {
        let mut projects = Projects::new();
        assert_eq!(projects.add("one"), Some(0));
        assert_eq!(projects.add("two"), Some(1));
        assert_eq!(projects.add("three"), Some(2));
        assert_eq!(projects.add("four"), None);
        assert_eq!(projects.projects.len(), MAX_PROJECTS);
    }

    #[test]
    fn removing_a_project_shifts_the_slots() {
        let mut projects = Projects::new();
        projects.add("one");
        projects.add("two");
        projects.set_active(1);
        assert!(projects.remove(0));
        assert_eq!(projects.projects.len(), 1);
        assert_eq!(projects.projects[0].name, "two");
        assert_eq!(projects.active(), 0);
    }

    #[test]
    fn removing_a_slot_that_does_not_exist_is_a_no_op() {
        let mut projects = Projects::new();
        assert!(!projects.remove(0));
        assert!(!projects.rename(0, "x"));
    }

    #[test]
    fn the_active_slot_is_clamped_to_one_that_exists() {
        let mut projects = Projects::new();
        projects.add("one");
        projects.active = 7;
        assert_eq!(projects.active(), 0);
        assert!(projects.active_project().is_some());
    }

    #[test]
    fn an_empty_set_has_no_active_project() {
        let projects = Projects::new();
        assert!(projects.active_project().is_none());
        assert!(projects.active_roots().is_empty());
        assert!(!projects.has_roots());
    }

    #[test]
    fn setting_an_active_slot_that_does_not_exist_is_refused() {
        let mut projects = Projects::new();
        projects.add("one");
        assert!(!projects.set_active(1));
        assert!(projects.set_active(0));
    }

    #[test]
    fn all_roots_deduplicates_across_projects() {
        let mut projects = Projects::new();
        let one = projects.add("one").unwrap();
        projects.projects[one].add_root("/shared");
        projects.projects[one].add_root("/one");
        let two = projects.add("two").unwrap();
        projects.projects[two].add_root("/shared");
        projects.projects[two].add_root("/two");
        assert_eq!(
            projects.all_roots(),
            vec![
                PathBuf::from("/shared"),
                PathBuf::from("/one"),
                PathBuf::from("/two")
            ]
        );
    }

    #[test]
    fn active_roots_are_only_the_active_project() {
        let mut projects = Projects::new();
        let one = projects.add("one").unwrap();
        projects.projects[one].add_root("/one");
        let two = projects.add("two").unwrap();
        projects.projects[two].add_root("/two");
        projects.set_active(1);
        assert_eq!(projects.active_roots(), &[PathBuf::from("/two")]);
    }

    #[test]
    fn a_project_round_trips_through_the_file() {
        let projects = sample();
        let text = projects.render();
        assert_eq!(Projects::parse(&text), projects);
    }

    #[test]
    fn the_file_names_the_active_slot() {
        let mut projects = sample();
        projects.add("second");
        projects.set_active(1);
        let text = projects.render();
        assert!(text.contains("active 1"));
        assert_eq!(Projects::parse(&text).active(), 1);
    }

    #[test]
    fn a_root_before_any_project_is_dropped() {
        let projects = Projects::parse("root /orphan\nproject p\nroot /kept\n");
        assert_eq!(projects.projects.len(), 1);
        assert_eq!(projects.projects[0].roots, vec![PathBuf::from("/kept")]);
    }

    #[test]
    fn a_fourth_project_in_the_file_is_ignored() {
        let text = "project a\nproject b\nproject c\nproject d\n";
        assert_eq!(Projects::parse(text).projects.len(), MAX_PROJECTS);
    }

    #[test]
    fn an_active_slot_past_the_end_falls_back_to_the_first() {
        let projects = Projects::parse("active 9\nproject a\n");
        assert_eq!(projects.active(), 0);
    }

    #[test]
    fn comments_and_blank_lines_are_skipped() {
        let projects = Projects::parse("# a comment\n\nproject p\n");
        assert_eq!(projects.projects.len(), 1);
    }

    #[test]
    fn an_unknown_keyword_is_ignored() {
        let projects = Projects::parse("project p\ncolour blue\nroot /a\n");
        assert_eq!(projects.projects[0].roots, vec![PathBuf::from("/a")]);
    }

    #[test]
    fn a_root_with_spaces_keeps_them() {
        let projects = Projects::parse("project p\nroot /a b/c\n");
        assert_eq!(projects.projects[0].roots, vec![PathBuf::from("/a b/c")]);
    }

    #[test]
    fn an_empty_file_gives_no_projects() {
        let projects = Projects::parse("");
        assert!(projects.projects.is_empty());
        assert!(!projects.has_roots());
    }
}
