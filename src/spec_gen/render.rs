use std::sync::LazyLock;

use serde::Serialize;
use tera::{Context, Tera};
use thiserror::Error;

use crate::model::PkgSpecFile;
use crate::source_spec;

static TEMPLATE: LazyLock<Tera> = LazyLock::new(|| {
    let mut tera = Tera::default();
    tera.add_raw_template("spec.tera", include_str!("template.tera"))
        .expect("failed to compile template.tera");
    tera.register_filter("rpm_escape", rpm_escape_filter);
    tera.register_filter("rpm_date", rpm_date_filter);
    tera
});

#[derive(Debug, Error)]
pub enum RenderError {
    #[error("failed to render spec template")]
    Template(#[from] tera::Error),
}

#[derive(Debug, Serialize)]
pub(crate) struct RenderablePackage<'a> {
    pub name: String,
    pub suffix: Option<&'a str>,
    pub summary: &'a str,
    pub description: &'a str,
    #[serde(flatten)]
    pub deps: &'a crate::model::DependencySet,
    #[serde(flatten)]
    pub files: &'a crate::model::FilesSpec,
    #[serde(flatten)]
    pub scriptlets: &'a crate::model::Scriptlets,
    pub is_base: bool,
    pub noarch: bool,
    pub license: Option<&'a str>,
    pub url: Option<&'a str>,
}

fn build_renderables<'a>(spec: &'a PkgSpecFile) -> Vec<RenderablePackage<'a>> {
    let mut packages = Vec::with_capacity(1 + spec.subpackages.len());

    packages.push(RenderablePackage {
        name: spec.package.name.clone(),
        suffix: None,
        summary: &spec.package.summary,
        description: &spec.package.description,
        deps: &spec.package.deps,
        files: &spec.package.files,
        scriptlets: &spec.package.scriptlets,
        is_base: true,
        noarch: spec.package.noarch,
        license: Some(&spec.package.license),
        url: spec.package.url.as_deref(),
    });

    for sub in &spec.subpackages {
        packages.push(RenderablePackage {
            name: format!("{}-{}", spec.package.name, sub.suffix),
            suffix: Some(&sub.suffix),
            summary: &sub.summary,
            description: &sub.description,
            deps: &sub.deps,
            files: &sub.files,
            scriptlets: &sub.scriptlets,
            is_base: false,
            noarch: sub.noarch.unwrap_or(false),
            license: sub.license.as_deref().or(Some(&spec.package.license)),
            url: sub.url.as_deref().or(spec.package.url.as_deref()),
        });
    }

    packages
}

fn render_build_section(spec: &PkgSpecFile) -> String {
    let mut parts = Vec::new();

    if let Some(macros) = spec.package.build.system.macros() {
        let extra = spec
            .package
            .build
            .system
            .extra_build_args_string(&spec.package.build.extra_build_args);

        if let Some(configure) = macros.configure {
            let mut line = configure.to_string();
            if !extra.is_empty() {
                line.push(' ');
                line.push_str(&extra);
            }
            parts.push(line);
            parts.push(macros.build.to_string());
        } else {
            let mut line = macros.build.to_string();
            if !extra.is_empty() {
                line.push(' ');
                line.push_str(&extra);
            }
            parts.push(line);
        }
    }

    if let Some(ref build_steps) = spec.package.build.steps.build {
        parts.push(build_steps.clone());
    }

    parts.join("\n")
}

fn render_install_section(spec: &PkgSpecFile) -> String {
    let mut parts = Vec::new();

    if let Some(macros) = spec.package.build.system.macros() {
        let extra = spec
            .package
            .build
            .system
            .extra_install_args_string(&spec.package.build.extra_install_args);
        let mut macro_line = macros.install.to_string();
        if !extra.is_empty() {
            macro_line.push(' ');
            macro_line.push_str(&extra);
        }
        parts.push(macro_line);
    }

    if let Some(ref install_steps) = spec.package.build.steps.install {
        parts.push(install_steps.clone());
    }

    parts.join("\n")
}

fn should_include_check(spec: &PkgSpecFile) -> bool {
    spec.package.build.steps.check.is_some() && spec.package.build.run_tests != Some(false)
}

fn rpm_escape_filter(
    value: &tera::Value,
    _args: &std::collections::HashMap<String, tera::Value>,
) -> tera::Result<tera::Value> {
    match value {
        tera::Value::String(s) => Ok(tera::Value::String(s.replace('%', "%%"))),
        _ => Ok(value.clone()),
    }
}

#[allow(deprecated)]
fn rpm_date_filter(
    value: &tera::Value,
    _args: &std::collections::HashMap<String, tera::Value>,
) -> tera::Result<tera::Value> {
    let s = match value {
        tera::Value::String(s) => s,
        _ => return Ok(value.clone()),
    };

    let date_fmt =
        time::format_description::parse("[year]-[month]-[day]")
            .map_err(|e| tera::Error::msg(format!("invalid date format: {e}")))?;

    let display_fmt =
        time::format_description::parse("[weekday repr:short] [month repr:short] [day padding:space] [year]")
            .map_err(|e| tera::Error::msg(format!("invalid display format: {e}")))?;

    let date = time::Date::parse(s, &date_fmt)
        .map_err(|_| tera::Error::msg(format!("failed to parse date: \"{s}\"")))?;

    let dt = time::PrimitiveDateTime::new(date, time::Time::MIDNIGHT);

    let formatted = dt
        .format(&display_fmt)
        .map_err(|e| tera::Error::msg(format!("failed to format date: {e}")))?;

    Ok(tera::Value::String(formatted))
}

pub fn render(spec: &PkgSpecFile, injected_build_deps: &[String]) -> Result<String, RenderError> {
    let renderables = build_renderables(spec);

    let mut build_requires: Vec<String> = spec
        .package
        .deps
        .build_depends
        .to_vec();
    for sub in &spec.subpackages {
        for dep in &sub.deps.build_depends {
            if !build_requires.contains(dep) {
                build_requires.push(dep.clone());
            }
        }
    }
    for dep in injected_build_deps {
        if !build_requires.contains(dep) {
            build_requires.push(dep.clone());
        }
    }

    let build_section = render_build_section(spec);
    let install_section = render_install_section(spec);
    let include_check = should_include_check(spec);
    let check_section = spec
        .package
        .build
        .steps
        .check
        .clone()
        .unwrap_or_default();

    let mut context = Context::new();

    context.insert("name", &spec.package.name);
    context.insert("version", &spec.package.version);
    context.insert("release", &spec.package.release);
    context.insert("epoch", &spec.package.epoch);
    context.insert("summary", &spec.package.summary);
    context.insert("license", &spec.package.license);
    context.insert("url", &spec.package.url);
    context.insert("group", &spec.package.group);
    context.insert("noarch", &spec.package.noarch);
    context.insert("description", &spec.package.description);

    let source_filenames: Vec<String> = spec
        .package
        .sources
        .iter()
        .map(|s| match source_spec::parse_source_entry(s) {
            source_spec::SourceEntry::Local { filename } => filename,
            source_spec::SourceEntry::Remote { filename, .. } => filename,
        })
        .collect();
    let patch_filenames: Vec<String> = spec
        .package
        .patches
        .iter()
        .map(|s| match source_spec::parse_source_entry(s) {
            source_spec::SourceEntry::Local { filename } => filename,
            source_spec::SourceEntry::Remote { filename, .. } => filename,
        })
        .collect();

    context.insert("sources", &source_filenames);
    context.insert("sha256sums", &spec.package.sha256sums);
    context.insert("patches", &patch_filenames);
    context.insert("patch_sha256sums", &spec.package.patch_sha256sums);

    context.insert("build_requires", &build_requires);
    context.insert("requires", &spec.package.deps.depends);
    context.insert("recommends", &spec.package.deps.recommends);
    context.insert("suggests", &spec.package.deps.suggests);
    context.insert("supplements", &spec.package.deps.supplements);
    context.insert("enhances", &spec.package.deps.enhances);
    context.insert("conflicts", &spec.package.deps.conflicts);
    context.insert("provides", &spec.package.deps.provides);
    context.insert("obsoletes", &spec.package.deps.obsoletes);

    context.insert("renderables", &renderables);

    context.insert("build_section", &build_section);
    context.insert("install_section", &install_section);
    context.insert("include_check", &include_check);
    context.insert("check_section", &check_section);

    context.insert("prep_steps", &spec.package.build.steps.prep);

    let use_default_setup = match &spec.package.build.steps.prep {
        Some(prep) => {
            let lower = prep.to_lowercase();
            !lower.contains("%setup") && !lower.contains("%autosetup")
        }
        None => true,
    };
    context.insert("use_default_setup", &use_default_setup);

    context.insert("changelog", &spec.package.changelog);

    let rendered = TEMPLATE.render("spec.tera", &context)?;
    Ok(rendered)
}
