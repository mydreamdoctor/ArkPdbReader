use std::collections::HashMap;
use std::path::Path;

use anyhow::Context;
use ms_pdb::syms::{Proc, RegRel, SymData, SymIter, SymKind};
use ms_pdb::tpi::TypeStream;
use ms_pdb::Pdb;

use crate::symbol_stream::{owner_lookup_key, parse_msvc_mangled_owner};
use crate::type_name::resolve_type_name;
use crate::types::ParamInfo;

#[derive(Debug, Clone, Default)]
pub struct ProcParamInfo {
    pub param_names: Vec<String>,
    pub param_type_names: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct ProcParamIndex {
    by_decorated_name: HashMap<String, ProcParamInfo>,
    by_owner_key: HashMap<String, Vec<String>>,
}

impl ProcParamIndex {
    pub fn get(&self, decorated_name: &str) -> Option<&ProcParamInfo> {
        self.by_decorated_name.get(decorated_name)
    }

    pub fn owner_candidates<'a>(&'a self, class_name: &str, method_name: &str) -> &'a [String] {
        let key = owner_lookup_key(class_name, method_name);
        self.by_owner_key
            .get(&key)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    fn insert(&mut self, decorated_name: String, info: ProcParamInfo) {
        if self.by_decorated_name.contains_key(&decorated_name) {
            return;
        }

        if let Some((class_name, method_name)) = parse_msvc_mangled_owner(&decorated_name) {
            let owner_key = owner_lookup_key(class_name, method_name);
            self.by_owner_key
                .entry(owner_key)
                .or_default()
                .push(decorated_name.clone());
        }

        self.by_decorated_name.insert(decorated_name, info);
    }
}

#[derive(Debug, Clone)]
struct CandidateMatch<'a> {
    decorated_name: &'a str,
    param_names: Vec<String>,
    score: usize,
    type_matches: usize,
    exact_count: bool,
}

pub fn build_proc_param_index(path: &str) -> anyhow::Result<ProcParamIndex> {
    let pdb = Pdb::open(Path::new(path))
        .with_context(|| format!("failed to reopen PDB for module symbol scan: {path}"))?;
    let type_stream = pdb
        .read_type_stream()
        .context("failed to read TPI stream for module symbol scan")?;
    let modules = pdb.modules().context("failed to read DBI module list")?;
    let public_symbol_names = build_public_proc_address_index(&pdb)
        .context("failed to read public symbol address map")?;

    let mut index = ProcParamIndex::default();

    for module in modules.iter() {
        let Some(module_stream) = pdb
            .read_module_stream(&module)
            .with_context(|| format!("failed to read module stream for {}", module.module_name))?
        else {
            continue;
        };

        let sym_data = match module_stream.sym_data() {
            Ok(data) => data,
            Err(_) => continue,
        };

        let mut iter = SymIter::new(sym_data);
        while let Some(sym) = iter.next() {
            if sym.kind != SymKind::S_GPROC32 && sym.kind != SymKind::S_LPROC32 {
                continue;
            }

            let proc = match sym.parse_as::<Proc>() {
                Ok(proc) => proc,
                Err(_) => {
                    skip_proc_scope(&mut iter);
                    continue;
                }
            };

            let decorated_name = public_symbol_names
                .get(&address_key(
                    proc.fixed.offset_segment.segment(),
                    proc.fixed.offset_segment.offset(),
                ))
                .cloned()
                .unwrap_or_else(|| proc.name.to_string());
            let info = collect_proc_params(&type_stream, &mut iter);
            index.insert(decorated_name, info);
        }
    }

    Ok(index)
}

pub fn choose_best_public_match(
    expected_params: &[ParamInfo],
    decorated_candidates: &[String],
    index: &ProcParamIndex,
) -> Option<(String, Vec<String>)> {
    choose_best_candidate(
        expected_params,
        decorated_candidates
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>()
            .as_slice(),
        index,
    )
    .map(|candidate| (candidate.decorated_name.to_owned(), candidate.param_names))
}

pub fn choose_best_owner_match(
    expected_params: &[ParamInfo],
    class_name: &str,
    method_name: &str,
    index: &ProcParamIndex,
) -> Option<Vec<String>> {
    let owner_candidates = index.owner_candidates(class_name, method_name);
    choose_best_candidate(
        expected_params,
        owner_candidates
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>()
            .as_slice(),
        index,
    )
    .map(|candidate| candidate.param_names)
}

fn choose_best_candidate<'a>(
    expected_params: &[ParamInfo],
    decorated_candidates: &'a [&'a str],
    index: &'a ProcParamIndex,
) -> Option<CandidateMatch<'a>> {
    let mut matches: Vec<CandidateMatch<'a>> = decorated_candidates
        .iter()
        .filter_map(|decorated_name| {
            let info = index.get(decorated_name)?;
            score_candidate(expected_params, info).map(
                |(score, type_matches, exact_count, param_names)| CandidateMatch {
                    decorated_name,
                    param_names,
                    score,
                    type_matches,
                    exact_count,
                },
            )
        })
        .collect();

    if matches.is_empty() {
        return None;
    }

    matches.sort_by(|left, right| {
        right
            .score
            .cmp(&left.score)
            .then_with(|| right.type_matches.cmp(&left.type_matches))
            .then_with(|| right.exact_count.cmp(&left.exact_count))
            .then_with(|| left.decorated_name.cmp(right.decorated_name))
    });

    if matches.len() == 1 {
        return matches.into_iter().next();
    }

    let best = &matches[0];
    let second = &matches[1];
    if best.score > second.score || (best.exact_count && best.type_matches == expected_params.len())
    {
        return matches.into_iter().next();
    }

    None
}

fn score_candidate(
    expected_params: &[ParamInfo],
    info: &ProcParamInfo,
) -> Option<(usize, usize, bool, Vec<String>)> {
    if expected_params.is_empty() {
        let exact_count = info.param_names.is_empty();
        let score = if exact_count { 16 } else { 1 };
        return Some((score, 0, exact_count, Vec::new()));
    }

    if info.param_names.len() < expected_params.len()
        || info.param_names.len() != info.param_type_names.len()
    {
        return None;
    }

    let exact_count = info.param_names.len() == expected_params.len();
    let mut best: Option<(usize, usize, Vec<String>)> = None;

    for start in 0..=info.param_names.len() - expected_params.len() {
        let candidate_names = &info.param_names[start..start + expected_params.len()];
        let candidate_types = &info.param_type_names[start..start + expected_params.len()];

        let mut type_matches = 0usize;
        let mut weak_matches = 0usize;

        for (expected, candidate_type) in expected_params.iter().zip(candidate_types.iter()) {
            if candidate_type == &expected.type_name {
                type_matches += 1;
            } else if candidate_type == "unknown" || expected.type_name == "unknown" {
                weak_matches += 1;
            }
        }

        let mut score = type_matches * 10 + weak_matches * 2;
        if exact_count {
            score += 5;
        }
        if start == 0 {
            score += 1;
        }

        let candidate = (score, type_matches, candidate_names.to_vec());
        if best.as_ref().map_or(true, |current| {
            candidate.0 > current.0 || (candidate.0 == current.0 && candidate.1 > current.1)
        }) {
            best = Some(candidate);
        }
    }

    best.map(|(score, type_matches, param_names)| (score, type_matches, exact_count, param_names))
}

fn collect_proc_params(type_stream: &TypeStream<Vec<u8>>, iter: &mut SymIter<'_>) -> ProcParamInfo {
    let mut params = ProcParamInfo::default();
    let mut regrel_fallback = ProcParamInfo::default();
    let mut scope_depth = 0u32;

    while let Some(sym) = iter.next() {
        if sym.kind.ends_scope() {
            if scope_depth == 0 {
                break;
            }
            scope_depth -= 1;
            continue;
        }

        if sym.kind.starts_scope() {
            scope_depth += 1;
            continue;
        }

        if scope_depth != 0 {
            continue;
        }

        if sym.kind == SymKind::S_LOCAL {
            let Ok(local) = sym.parse_as::<ms_pdb::syms::Local>() else {
                continue;
            };

            let is_param = (local.fixed.flags.get() & 1) != 0;
            if !is_param {
                continue;
            }

            params.param_names.push(local.name.to_string());
            params
                .param_type_names
                .push(resolve_type_name(type_stream, local.fixed.ty.get(), 0));
            continue;
        }

        if sym.kind == SymKind::S_REGREL32 {
            let Ok(regrel) = sym.parse_as::<RegRel>() else {
                continue;
            };

            regrel_fallback.param_names.push(regrel.name.to_string());
            regrel_fallback.param_type_names.push(resolve_type_name(
                type_stream,
                regrel.fixed.ty.get(),
                0,
            ));
        }
    }

    if params.param_names.is_empty() && !regrel_fallback.param_names.is_empty() {
        regrel_fallback
    } else {
        params
    }
}

fn build_public_proc_address_index(pdb: &Pdb) -> anyhow::Result<HashMap<u64, String>> {
    let gss_data = pdb.read_gss()?.stream_data;
    let mut index = HashMap::with_capacity(65536);

    for sym in SymIter::new(&gss_data) {
        if sym.kind != SymKind::S_PUB32 && sym.kind != SymKind::S_PUB32_ST {
            continue;
        }

        let Ok(SymData::Pub(public_sym)) = sym.parse() else {
            continue;
        };

        let name = match std::str::from_utf8(public_sym.name.as_ref()) {
            Ok(name) if name.starts_with('?') => name,
            _ => continue,
        };

        index
            .entry(address_key(
                public_sym.fixed.offset_segment.segment(),
                public_sym.fixed.offset_segment.offset(),
            ))
            .or_insert_with(|| name.to_owned());
    }

    Ok(index)
}

fn address_key(segment: u16, offset: u32) -> u64 {
    ((segment as u64) << 32) | offset as u64
}

fn skip_proc_scope(iter: &mut SymIter<'_>) {
    let mut scope_depth = 0u32;

    while let Some(sym) = iter.next() {
        if sym.kind.ends_scope() {
            if scope_depth == 0 {
                break;
            }
            scope_depth -= 1;
            continue;
        }

        if sym.kind.starts_scope() {
            scope_depth += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn param(type_name: &str) -> ParamInfo {
        ParamInfo {
            name: "param0".to_owned(),
            type_name: type_name.to_owned(),
        }
    }

    #[test]
    fn exact_type_match_beats_same_arity_overload() {
        let mut index = ProcParamIndex::default();
        index.insert(
            "?Foo@Bar@@QEAAXH@Z".to_owned(),
            ProcParamInfo {
                param_names: vec!["value".to_owned()],
                param_type_names: vec!["int".to_owned()],
            },
        );
        index.insert(
            "?Foo@Bar@@QEAAXPEBD@Z".to_owned(),
            ProcParamInfo {
                param_names: vec!["text".to_owned()],
                param_type_names: vec!["char const*".to_owned()],
            },
        );

        let matched = choose_best_public_match(
            &[param("char const*")],
            &[
                "?Foo@Bar@@QEAAXH@Z".to_owned(),
                "?Foo@Bar@@QEAAXPEBD@Z".to_owned(),
            ],
            &index,
        )
        .expect("expected a best match");

        assert_eq!(matched.0, "?Foo@Bar@@QEAAXPEBD@Z");
        assert_eq!(matched.1, vec!["text"]);
    }

    #[test]
    fn owner_lookup_can_skip_leading_hidden_param() {
        let mut index = ProcParamIndex::default();
        index.insert(
            "?PreLogin@AGameModeBase@@UEAAXAEBVFString@@0PEBVFUniqueNetIdRepl@@AEAV2@@Z".to_owned(),
            ProcParamInfo {
                param_names: vec![
                    "__formal".to_owned(),
                    "Options".to_owned(),
                    "Address".to_owned(),
                    "UniqueId".to_owned(),
                    "ErrorMessage".to_owned(),
                ],
                param_type_names: vec![
                    "unknown".to_owned(),
                    "FString const&".to_owned(),
                    "FString const&".to_owned(),
                    "FUniqueNetIdRepl const*".to_owned(),
                    "FString&".to_owned(),
                ],
            },
        );

        let matched = choose_best_owner_match(
            &[
                param("FString const&"),
                param("FString const&"),
                param("FUniqueNetIdRepl const*"),
                param("FString&"),
            ],
            "AGameModeBase",
            "PreLogin",
            &index,
        )
        .expect("expected owner-based parameter names");

        assert_eq!(
            matched,
            vec!["Options", "Address", "UniqueId", "ErrorMessage"]
        );
    }
}
