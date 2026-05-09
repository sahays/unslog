use serde::{Deserialize, Serialize};

/// Canonical role taxonomy. The freeform `Company.role` field still captures
/// the company-specific name (e.g. "Applied AI Solutions Architect", "Customer
/// Engineer") — this enum just lets the curator filter and pool questions
/// across companies for the same canonical track.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    SolutionsArchitect,
    SoftwareEngineer,
    EngineeringManager,
    ProgramManager,
    ProductManager,
}

impl Role {
    pub const ALL: &'static [Role] = &[
        Role::SolutionsArchitect,
        Role::SoftwareEngineer,
        Role::EngineeringManager,
        Role::ProgramManager,
        Role::ProductManager,
    ];

    pub fn as_str(&self) -> &'static str {
        match self {
            Role::SolutionsArchitect => "solutions_architect",
            Role::SoftwareEngineer => "software_engineer",
            Role::EngineeringManager => "engineering_manager",
            Role::ProgramManager => "program_manager",
            Role::ProductManager => "product_manager",
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            Role::SolutionsArchitect => "Solutions Architect",
            Role::SoftwareEngineer => "Software Engineer",
            Role::EngineeringManager => "Engineering Manager",
            Role::ProgramManager => "Program Manager",
            Role::ProductManager => "Product Manager",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "solutions_architect" => Some(Role::SolutionsArchitect),
            "software_engineer" => Some(Role::SoftwareEngineer),
            "engineering_manager" => Some(Role::EngineeringManager),
            "program_manager" => Some(Role::ProgramManager),
            "product_manager" => Some(Role::ProductManager),
            _ => None,
        }
    }
}

impl std::fmt::Display for Role {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.display_name())
    }
}
