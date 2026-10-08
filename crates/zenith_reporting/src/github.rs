//! GitHub Actions workflow commands, which show up as annotations on pull
//! requests: `::error file=…,line=…,col=…,title=…::message`.

use std::io;

use zenith_config::Severity;

use crate::{RenderContext, findings};

pub(crate) fn render(context: &RenderContext, out: &mut dyn io::Write) -> io::Result<()> {
    for contract in &context.report.contracts {
        let command = match contract.severity {
            Severity::Error => "error",
            Severity::Warn => "warning",
        };
        let title = format!("zenith({})", contract.id);
        for finding in findings(contract) {
            let mut properties = Vec::new();
            if let Some(link) = finding.link
                && let Some(import) = link.imports.first()
            {
                properties.push(format!("file={}", escape_property(&link.importer)));
                properties.push(format!("line={}", import.line));
                properties.push(format!("col={}", import.column));
            }
            properties.push(format!("title={}", escape_property(&title)));
            let mut message = finding.message;
            if let Some(guidance) = &contract.guidance {
                message.push('\n');
                message.push_str(guidance.trim());
            }
            writeln!(out, "::{command} {}::{}", properties.join(","), escape_data(&message))?;
        }
        for warning in &contract.warnings {
            writeln!(out, "::warning title={}::{}", escape_property(&title), escape_data(warning))?;
        }
    }
    for unused in &context.report.unused_suppressions {
        writeln!(
            out,
            "::warning file={},line={},col={},title=zenith::{}",
            escape_property(&unused.file),
            unused.line,
            unused.column,
            escape_data(&format!("unused suppression: {}", unused.reason))
        )?;
    }
    Ok(())
}

/// Escape a workflow command message.
fn escape_data(value: &str) -> String {
    value.replace('%', "%25").replace('\r', "%0D").replace('\n', "%0A")
}

/// Escape a workflow command property value.
fn escape_property(value: &str) -> String {
    escape_data(value).replace(':', "%3A").replace(',', "%2C")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_workflow_commands() {
        assert_eq!(escape_data("a%b\nc"), "a%25b%0Ac");
        assert_eq!(escape_property("zenith(a,b):c"), "zenith(a%2Cb)%3Ac");
    }
}
