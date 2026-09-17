//! Structured agent output types (JSON-schema'd reports).

pub mod code;
pub mod lenient;
pub mod relationship;
pub mod research;

pub use code::{
    CodePurpose, Dependency, DirectoryDossier, DirectoryPurpose, DirectorySummaryResponse,
    FileInsight, InterfaceInfo, ParameterInfo, classify_directory_purpose,
};
pub use relationship::{ArchitectureLayer, CoreDependency, DependencyType, RelationshipAnalysis};
pub use research::{
    APIBoundary, BoundaryAnalysisReport, BusinessFlow, BusinessFlowStep, CLIArgument, CLIBoundary,
    CLIOption, DataFlow, DatabaseFunction, DatabaseOverviewReport, DatabaseProject, DatabaseTable,
    DatabaseView, DomainModule, DomainModulesReport, DomainRelation, ExternalSystem,
    IntegrationSuggestion, KeyModuleReport, ProcedureParameter, ProjectType, RouterBoundary,
    RouterParam, StoredProcedure, SubModule, SystemBoundary, SystemContextReport, TableColumn,
    TableRelationship, UserPersona,
};
