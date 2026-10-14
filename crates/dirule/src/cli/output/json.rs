use super::{ApplySummary, CURRENT_OUTPUT_VERSION};
use dirule_core::{Conflict, Finding, Plan, PlannedEntry};
use serde::Serialize;
use serde::ser::{SerializeSeq, SerializeStruct, Serializer};
use std::error::Error;
use std::fmt;
use std::io::{self, Write};
use std::path::Path;

pub(super) fn write_plan(target: &Path, plan: &Plan) -> Result<(), Box<dyn Error>> {
    write_document(&PlanDocument { target, plan })
}

pub(super) fn write_apply(
    target: &Path,
    plan: &Plan,
    summary: &ApplySummary<'_>,
) -> Result<(), Box<dyn Error>> {
    let document = ApplyDocument {
        execution: ExecutionDocument {
            moved: summary.moved,
            run_id: summary.run_id,
            skipped: summary.skipped,
            status: summary.status.as_str(),
        },
        plan: PlanDocument { target, plan },
        schema_version: CURRENT_OUTPUT_VERSION,
    };

    write_document(&document)
}

fn write_document<T: Serialize>(document: &T) -> Result<(), Box<dyn Error>> {
    let stdout = io::stdout();
    let mut writer = stdout.lock();

    serde_json::to_writer_pretty(&mut writer, document)?;
    writer.write_all(b"\n")?;

    Ok(())
}

struct PlanDocument<'a> {
    target: &'a Path,
    plan: &'a Plan,
}

impl Serialize for PlanDocument<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut document = serializer.serialize_struct("PlanDocument", 6)?;
        document.serialize_field("conflicts", &ConflictList(&self.plan.conflicts))?;
        document.serialize_field("diagnostics", &DiagnosticList(&self.plan.findings))?;
        document.serialize_field("entries", &EntryList(&self.plan.entries))?;
        document.serialize_field("findings", &FindingMessages(&self.plan.findings))?;
        document.serialize_field("schema_version", CURRENT_OUTPUT_VERSION)?;
        document.serialize_field("target", &TargetPath(self.target))?;
        document.end()
    }
}

struct DiagnosticList<'a>(&'a [Finding]);

impl Serialize for DiagnosticList<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut diagnostics = serializer.serialize_seq(Some(self.0.len()))?;
        for finding in self.0 {
            diagnostics.serialize_element(&DiagnosticDocument(finding))?;
        }
        diagnostics.end()
    }
}

struct DiagnosticDocument<'a>(&'a Finding);

impl Serialize for DiagnosticDocument<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let finding = self.0;
        let mut document = serializer.serialize_struct("DiagnosticDocument", 4)?;
        document.serialize_field("kind", &finding.kind)?;
        document.serialize_field("path", &finding.path)?;
        document.serialize_field("message", &finding.message)?;
        document.serialize_field("suggestion", finding.kind.suggestion())?;
        document.end()
    }
}

struct FindingMessages<'a>(&'a [Finding]);

impl Serialize for FindingMessages<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut messages = serializer.serialize_seq(Some(self.0.len()))?;
        for finding in self.0 {
            messages.serialize_element(&finding.message)?;
        }
        messages.end()
    }
}

#[derive(Serialize)]
struct ApplyDocument<'a> {
    execution: ExecutionDocument<'a>,
    plan: PlanDocument<'a>,
    schema_version: &'static str,
}

#[derive(Serialize)]
struct ExecutionDocument<'a> {
    moved: usize,
    run_id: Option<&'a str>,
    skipped: usize,
    status: &'a str,
}

struct EntryList<'a>(&'a [PlannedEntry]);

impl Serialize for EntryList<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut entries = serializer.serialize_seq(Some(self.0.len()))?;
        for entry in self.0 {
            entries.serialize_element(&EntryDocument(entry))?;
        }
        entries.end()
    }
}

struct EntryDocument<'a>(&'a PlannedEntry);

impl Serialize for EntryDocument<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let entry = self.0;
        let mut document = serializer.serialize_struct("EntryDocument", 4)?;
        document.serialize_field("destination", &PortablePath(&entry.destination))?;
        document.serialize_field("kind", &entry.kind)?;
        document.serialize_field("rule", &entry.rule)?;
        document.serialize_field("source", &PortablePath(&entry.source))?;
        document.end()
    }
}

struct ConflictList<'a>(&'a [Conflict]);

impl Serialize for ConflictList<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut conflicts = serializer.serialize_seq(Some(self.0.len()))?;
        for conflict in self.0 {
            conflicts.serialize_element(&ConflictDocument(conflict))?;
        }
        conflicts.end()
    }
}

struct ConflictDocument<'a>(&'a Conflict);

impl Serialize for ConflictDocument<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut document = serializer.serialize_struct("ConflictDocument", 5)?;
        document.serialize_field("kind", &self.0.kind)?;
        document.serialize_field("path", &PortablePath(&self.0.path))?;
        let related_paths = self
            .0
            .related_paths
            .iter()
            .map(|path| PortablePath(path).to_string())
            .collect::<Vec<_>>();
        document.serialize_field("related_paths", &related_paths)?;
        document.serialize_field("reason", &self.0.reason)?;
        document.serialize_field("suggestion", self.0.kind.suggestion())?;
        document.end()
    }
}

struct TargetPath<'a>(&'a Path);

impl Serialize for TargetPath<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.0.to_string_lossy())
    }
}

struct PortablePath<'a>(&'a Path);

impl Serialize for PortablePath<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.collect_str(self)
    }
}

impl fmt::Display for PortablePath<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (index, component) in self.0.components().enumerate() {
            if index > 0 {
                formatter.write_str("/")?;
            }

            write!(formatter, "{}", component.as_os_str().to_string_lossy())?;
        }

        Ok(())
    }
}
