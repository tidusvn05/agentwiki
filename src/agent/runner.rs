//! `AgentRunner` — the agentic-loop engine: render prompt → cache →
//! quota → backend → parse → retry-with-feedback → fallback model.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use futures_util::{StreamExt, stream::FuturesUnordered};
use serde_json::Value;

use super::context::ResearchContext;
use super::materials;
use super::registry;
use super::spec::{self, AgentSpec, ExecKind, FanTarget, Material};
use crate::backend::{AgentRequest, BackendKind, TokenUsage, tail};
use crate::cache::Cache;
use crate::config::{Config, Mode, ModelTier};
use crate::error::{Error, Result};
use crate::pipeline::PipelineCtx;
use crate::prompt::render;
use crate::quota::CallRecord;

/// Run one spec: expand fan-out, run instances, aggregate into ctx.
pub async fn run_spec(spec: &AgentSpec, pctx: &Arc<PipelineCtx>) -> Result<()> {
    let started = Instant::now();
    if let ExecKind::Deterministic(f) = spec.exec {
        pctx.progress.add_total(1);
        pctx.progress.start(spec.name);
        let dep = first_dep(spec, pctx).await;
        let md = f(&pctx.scan, &pctx.config, &dep)?;
        pctx.ctx.insert(spec.name, Value::String(md)).await;
        pctx.stats.lock().await.record(spec.name, started.elapsed());
        pctx.progress.finish(spec.name);
        return Ok(());
    }

    match spec.fan_out {
        None => {
            pctx.progress.add_total(1);
            let v = run_instance(spec, spec.name, None, pctx).await?;
            pctx.ctx.insert(spec.name, v).await;
        }
        Some(fan) => {
            let targets = spec::expand(fan, spec, pctx).await;
            pctx.progress.add_total(targets.len() as u64);
            if targets.is_empty() {
                // E.g. domain_modules produced no domains — store an empty
                // result so dependents see a valid (empty) context.
                pctx.ctx
                    .insert(spec.name, Value::Object(Default::default()))
                    .await;
                return Ok(());
            }
            // FuturesUnordered, not JoinSet: these futures live inside this
            // task, so aborting it drops them synchronously — which drops
            // each in-flight `Child` and kills the CLI (kill_on_drop).
            let mut pending = FuturesUnordered::new();
            for t in &targets {
                let (spec, key) = (spec.clone(), spec.instance_key(Some(&t.key)));
                pending.push(async move {
                    run_instance(&spec, &key, Some(t), pctx)
                        .await
                        .map(|v| (t, v))
                });
            }
            let mut results = Vec::new();
            while let Some(r) = pending.next().await {
                let (t, v) = r?;
                results.push((t.clone(), v));
            }
            aggregate(spec, results, pctx).await?;
        }
    }
    pctx.stats.lock().await.record(spec.name, started.elapsed());
    Ok(())
}

/// Merge fan-out results into the context under `spec.name`.
async fn aggregate(
    spec: &AgentSpec,
    mut results: Vec<(FanTarget, Value)>,
    pctx: &Arc<PipelineCtx>,
) -> Result<()> {
    results.sort_by(|a, b| a.0.key.cmp(&b.0.key));
    match spec.name {
        "dir_summary" => {
            let mut dossiers = Vec::new();
            for (t, v) in results {
                let resp: crate::agent::reports::DirectorySummaryResponse =
                    serde_json::from_value(v).map_err(|e| Error::Parse {
                        agent: t.key.clone(),
                        message: e.to_string(),
                    })?;
                let dir = t
                    .dir
                    .ok_or_else(|| Error::Pipeline("dir_summary target missing dir".to_string()))?;
                dossiers.push(materials::dossier_from(&dir, &resp));
            }
            let v = serde_json::to_value(&dossiers).map_err(|e| Error::Pipeline(e.to_string()))?;
            pctx.ctx.insert(spec.name, v).await;
        }
        _ => {
            // PerDomain: store `{domain_name: result}` map.
            let mut map = serde_json::Map::new();
            for (t, mut v) in results {
                // The model doesn't reliably echo the domain; stamp it.
                if spec.name == "key_module"
                    && let Some(obj) = v.as_object_mut()
                {
                    obj.insert("domain_name".to_string(), Value::String(t.key.clone()));
                }
                map.insert(t.key, v);
            }
            pctx.ctx.insert(spec.name, Value::Object(map)).await;
        }
    }
    Ok(())
}

/// First dep's stored result (used by deterministic specs).
async fn first_dep(spec: &AgentSpec, pctx: &Arc<PipelineCtx>) -> Value {
    match spec.deps.first() {
        Some(d) => pctx.ctx.get(d).await.unwrap_or(Value::Null),
        None => Value::Null,
    }
}

/// One agent invocation, tracked on the progress bar.
async fn run_instance(
    spec: &AgentSpec,
    key: &str,
    target: Option<&FanTarget>,
    pctx: &Arc<PipelineCtx>,
) -> Result<Value> {
    pctx.progress.start(key);
    let result = run_instance_inner(spec, key, target, pctx).await;
    pctx.progress.finish(key);
    result
}

/// The invocation itself: cache → quota → backend → validate → retry.
async fn run_instance_inner(
    spec: &AgentSpec,
    key: &str,
    target: Option<&FanTarget>,
    pctx: &Arc<PipelineCtx>,
) -> Result<Value> {
    let prompt = build_prompt(spec, target, pctx).await?;
    let model_str = pctx.config.model_for(spec.tier).to_string();
    let (kind, model) = BackendKind::parse(&model_str)?;
    let backend = pctx.backend(kind)?;
    let cwd = match pctx.config.mode {
        Mode::Agentic => pctx.scan.root.clone(),
        Mode::Embedded => pctx.empty_cwd.clone(),
    };

    let cache_key = Cache::key(&prompt, &model_str, kind.as_str());
    if let Some(hit) = pctx.cache.get(&cache_key) {
        match parse_output(spec, &hit.text, key) {
            Ok(v) => {
                pctx.stats.lock().await.cache_hit(hit.meta.secs);
                tracing::debug!(agent = key, "cache hit");
                return Ok(v);
            }
            Err(_) => tracing::debug!(agent = key, "stale cache entry, regenerating"),
        }
    }

    // Bound concurrent CLI calls across all fan-out instances; cancel
    // unwinds promptly even while queued behind the semaphore.
    let _permit = tokio::select! {
        biased;
        p = pctx.semaphore.acquire() => p
            .map_err(|e| Error::Pipeline(format!("semaphore: {e}")))?,
        () = pctx.cancel.cancelled() => return Err(Error::Cancelled),
    };

    let mut feedback = String::new();
    let mut last_err = Error::Pipeline("no attempts".to_string());
    let json_schema = spec.schema.map(|s| (s.json_schema)());
    for attempt in 0..=pctx.config.limits.retry_attempts {
        if pctx.cancel.is_cancelled() {
            return Err(Error::Cancelled);
        }
        pctx.quota.consume().await?;
        let req = AgentRequest {
            prompt: format!("{prompt}{feedback}"),
            cwd: cwd.clone(),
            model: model.clone(),
            timeout: pctx.config.call_timeout(),
            agent: key.to_string(),
            json_schema: json_schema.clone(),
        };
        let started = Instant::now();
        // Biased: a response that just landed still gets cached even if a
        // cancel arrived in the same instant.
        let response = tokio::select! {
            biased;
            r = backend.run(req) => r,
            () = pctx.cancel.cancelled() => return Err(Error::Cancelled),
        };
        let (status, err, usage) = match response {
            Ok(r) => match parse_output(spec, &r.text, key) {
                Ok(v) => {
                    pctx.cache.put(
                        &cache_key,
                        &r.text,
                        crate::cache::CacheMeta {
                            agent: key.to_string(),
                            backend: kind.as_str().to_string(),
                            model: model_str.clone(),
                            created_at: crate::quota::now_rfc3339(),
                            secs: r.duration.as_secs_f64(),
                        },
                    )?;
                    pctx.stats.lock().await.cli_call();
                    record(
                        pctx,
                        key,
                        kind,
                        &model_str,
                        CallMetrics {
                            prompt_chars: prompt.len(),
                            secs: started.elapsed(),
                            status: "ok",
                            usage: r.usage,
                        },
                    )
                    .await;
                    return Ok(v);
                }
                Err(e) => ("validation", e, r.usage),
            },
            Err(e) => ("error", e, None),
        };
        record(
            pctx,
            key,
            kind,
            &model_str,
            CallMetrics {
                prompt_chars: prompt.len(),
                secs: started.elapsed(),
                status,
                usage,
            },
        )
        .await;
        pctx.stats.lock().await.cli_call();
        tracing::debug!(agent = key, attempt, error = %err, "attempt failed");
        feedback = format!(
            "\n\n**RETRY**: Your previous response failed: {err}. \
             Correct it and return ONLY the required output."
        );
        last_err = err;
    }

    // Fallback: Efficient-tier agents retry once on the Powerful model.
    if spec.tier != ModelTier::Efficient || pctx.cancel.is_cancelled() {
        return Err(last_err);
    }
    let fb_str = pctx.config.model_for(ModelTier::Powerful).to_string();
    let Ok((fk, fm)) = BackendKind::parse(&fb_str) else {
        return Err(last_err);
    };
    let Ok(fb) = pctx.backend(fk) else {
        return Err(last_err);
    };
    pctx.quota.consume().await?;
    let req = AgentRequest {
        prompt: format!("{prompt}{feedback}"),
        cwd,
        model: fm,
        timeout: pctx.config.call_timeout(),
        agent: key.to_string(),
        json_schema: json_schema.clone(),
    };
    if let Ok(r) = fb.run(req).await
        && let Ok(v) = parse_output(spec, &r.text, key)
    {
        pctx.cache.put(
            &cache_key,
            &r.text,
            crate::cache::CacheMeta {
                agent: key.to_string(),
                backend: fk.as_str().to_string(),
                model: fb_str,
                created_at: crate::quota::now_rfc3339(),
                secs: r.duration.as_secs_f64(),
            },
        )?;
        return Ok(v);
    }
    Err(last_err)
}

/// Render the full prompt for one instance.
async fn build_prompt(
    spec: &AgentSpec,
    target: Option<&FanTarget>,
    pctx: &Arc<PipelineCtx>,
) -> Result<String> {
    let tmpl = pctx.prompts.load(spec.prompt_tmpl)?;
    let mut vars: HashMap<&str, String> = HashMap::new();
    let materials = build_materials(spec, pctx).await;
    let cap = pctx.config.limits.materials_char_cap;
    vars.insert("materials", materials.chars().take(cap).collect::<String>());
    vars.insert("custom", custom_block(spec, target, pctx).await);
    vars.insert(
        "language_instruction",
        pctx.config.target_language.instruction().to_string(),
    );
    vars.insert("schema_block", schema_block(spec));
    vars.insert("agentic_note", agentic_note(&pctx.config));
    Ok(render(&tmpl, &vars))
}

/// `{{materials}}` — dep results + scan materials.
async fn build_materials(spec: &AgentSpec, pctx: &Arc<PipelineCtx>) -> String {
    let mut s = String::new();
    // Dep results first — they're the freshest context.
    for dep in spec.deps {
        if let Some(v) = pctx.ctx.get(dep).await {
            s.push_str(&materials::dep_block(registry::display_name(dep), &v));
        }
    }
    if pctx.config.mode == Mode::Agentic {
        return s;
    }
    let dossiers = dossiers(&pctx.ctx).await;
    for m in spec.materials {
        match m {
            Material::CodeInsights => {
                s.push_str(&materials::code_insights_block(
                    &dossiers,
                    pctx.config.limits.code_insights_limit,
                ));
            }
            Material::Relationships => {
                let rel: Option<crate::agent::reports::RelationshipAnalysis> =
                    pctx.ctx.get_typed("relationships").await;
                s.push_str(&materials::relationships_block(rel.as_ref()));
            }
            other => {
                if let Some(b) = materials::render_material(*other, &pctx.scan) {
                    s.push_str(&b);
                }
            }
        }
    }
    s
}

/// `{{custom}}` — per-instance data block.
async fn custom_block(
    spec: &AgentSpec,
    target: Option<&FanTarget>,
    pctx: &Arc<PipelineCtx>,
) -> String {
    let dossiers = dossiers(&pctx.ctx).await;
    match (spec.name, target) {
        ("dir_summary", Some(t)) => t
            .dir
            .as_ref()
            .map(|d| materials::dir_summary_custom(d, pctx.config.limits.file_source_chars))
            .unwrap_or_default(),
        ("relationships", _) => materials::relationships_custom(&dossiers),
        ("key_module", Some(t)) => t
            .domain
            .as_ref()
            .map(|d| materials::key_module_custom(d, &dossiers))
            .unwrap_or_default(),
        ("deep_dive", Some(t)) => t
            .domain
            .as_ref()
            .map(|d| {
                format!(
                    "**Module**: {}\n**Description**: {}\n**Code paths**: {}\n",
                    d.name,
                    d.description,
                    d.code_paths.join(", ")
                )
            })
            .unwrap_or_default(),
        ("boundary", _) => {
            materials::boundary_custom(&dossiers, pctx.config.limits.code_insights_limit)
        }
        ("database", _) => materials::database_custom(&dossiers),
        _ => String::new(),
    }
}

/// Stored dossiers (empty before dir_summary completes).
async fn dossiers(ctx: &ResearchContext) -> Vec<crate::agent::reports::DirectoryDossier> {
    ctx.get_typed("dir_summary").await.unwrap_or_default()
}

/// Validate a backend response against the spec's contract.
fn parse_output(spec: &AgentSpec, text: &str, agent: &str) -> Result<Value> {
    match &spec.schema {
        Some(schema) => {
            let v = extract_json(text, agent)?;
            (schema.validate)(v, agent)
        }
        None => {
            if text.trim().is_empty() {
                return Err(Error::Parse {
                    agent: agent.to_string(),
                    message: "empty response".to_string(),
                });
            }
            Ok(Value::String(text.trim().to_string()))
        }
    }
}

/// Lenient JSON extraction, ported from deepwiki-rs:
/// strict parse → fenced `json` block → depth-counted first `{…}` object.
pub fn extract_json(text: &str, agent: &str) -> Result<Value> {
    let trimmed = text.trim();
    if let Ok(v) = serde_json::from_str::<Value>(trimmed) {
        return Ok(v);
    }
    // ```json … ``` fence
    if let Some(start) = trimmed.find("```json") {
        let body = &trimmed[start + 7..];
        if let Some(end) = body.find("```")
            && let Ok(v) = serde_json::from_str::<Value>(body[..end].trim())
        {
            return Ok(v);
        }
    }
    // Depth-counted first object/array, honoring strings/escapes.
    if let Some(start) = trimmed.find(['{', '[']) {
        let bytes = trimmed.as_bytes();
        let (mut depth, mut in_str, mut esc) = (0i32, false, false);
        for (i, &b) in bytes.iter().enumerate().skip(start) {
            match b {
                b'\\' if in_str => esc = !esc,
                b'"' if !esc => in_str = !in_str,
                b'{' | b'[' if !in_str => depth += 1,
                b'}' | b']' if !in_str => {
                    depth -= 1;
                    if depth == 0 {
                        if let Ok(v) = serde_json::from_str::<Value>(&trimmed[start..=i]) {
                            return Ok(v);
                        }
                        break;
                    }
                }
                _ => esc = false,
            }
        }
    }
    Err(Error::Parse {
        agent: agent.to_string(),
        message: format!("no valid JSON in response: {}", tail(text, 200)),
    })
}

/// `{{schema_block}}` — JSON contract injected for structured agents.
fn schema_block(spec: &AgentSpec) -> String {
    match &spec.schema {
        Some(s) => {
            let schema = (s.json_schema)();
            format!(
                "Your response MUST be a single valid JSON object matching this JSON Schema:\n\
                 ```json\n{}\n```\n\
                 Do not include any text before or after the JSON. Do not wrap the JSON in \
                 markdown code fences. All fields required by the schema must be present.",
                serde_json::to_string_pretty(&schema).unwrap_or_default()
            )
        }
        None => String::new(),
    }
}

/// `{{agentic_note}}` — only in agentic mode: the repo is the cwd.
fn agentic_note(config: &Config) -> String {
    if config.mode == Mode::Agentic {
        "**Agentic mode**: the project repository is your current working directory. \
         Explore it READ-ONLY — do not modify any files."
            .to_string()
    } else {
        String::new()
    }
}

/// Metrics for one `calls.jsonl` record.
struct CallMetrics {
    prompt_chars: usize,
    secs: std::time::Duration,
    status: &'static str,
    usage: Option<TokenUsage>,
}

/// Append to `calls.jsonl` (best-effort).
async fn record(pctx: &PipelineCtx, agent: &str, kind: BackendKind, model: &str, m: CallMetrics) {
    pctx.quota
        .record(CallRecord {
            ts: crate::quota::now_rfc3339(),
            agent,
            backend: kind.as_str(),
            model,
            prompt_chars: m.prompt_chars,
            secs: m.secs.as_secs_f64(),
            status: m.status,
            input_tokens: m.usage.as_ref().map(|u| u.input),
            output_tokens: m.usage.as_ref().map(|u| u.output),
        })
        .await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_strict() {
        let v = extract_json("{\"a\": 1}", "t").unwrap();
        assert_eq!(v["a"], 1);
    }

    #[test]
    fn extract_fenced() {
        let v = extract_json("Here:\n```json\n{\"a\": 2}\n```\nThanks", "t").unwrap();
        assert_eq!(v["a"], 2);
    }

    #[test]
    fn extract_prose_wrapped() {
        let v = extract_json("Sure! {\"a\": {\"b\": \"x{y}\"}} done", "t").unwrap();
        assert_eq!(v["a"]["b"], "x{y}");
    }

    #[test]
    fn extract_fails_on_prose() {
        assert!(extract_json("no json here", "t").is_err());
    }
}
