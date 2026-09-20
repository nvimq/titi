//! Credential ladder: runtime > config > OAuth > login-key > env > stored >
//! fallback. The model-facing [`Credential`] type carries only the access
//! material — refresh tokens are typologically hidden from this layer and
//! owned exclusively by the auth actor.

use smol_str::SmolStr;

/// Ladder rungs in strict priority order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LadderLevel {
    /// 1. `--api-key` runtime override; never persisted.
    Runtime = 1,
    /// 2. `models.yml` config key (beats OAuth so a proxy never receives an
    /// upstream OAuth token).
    Config = 2,
    /// 3. Stored OAuth access token.
    OAuth = 3,
    /// 4. Key saved by an interactive `/login`.
    LoginKey = 4,
    /// 5. Provider env var (including `.env` files).
    Env = 5,
    /// 6. Other stored keys (broker-migrated).
    Stored = 6,
    /// 7. Fallback resolver in the descriptor.
    Fallback = 7,
}

/// The resolved access material. Deliberately has **no** refresh field:
/// refresh tokens never cross the credential-actor boundary into the model
/// layer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Credential {
    pub access: SmolStr,
    pub kind: CredKind,
    /// Which rung produced this credential (for telemetry/rotation).
    pub level: LadderLevel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredKind {
    ApiKey,
    BearerToken,
}

/// All ladder inputs for one resolution.
#[derive(Debug, Clone, Default)]
pub struct LadderCtx {
    pub runtime_override: Option<SmolStr>,
    pub config_key: Option<SmolStr>,
    pub oauth_token: Option<SmolStr>,
    pub login_key: Option<SmolStr>,
    pub env_key: Option<SmolStr>,
    pub stored_key: Option<SmolStr>,
    pub fallback_key: Option<SmolStr>,
}

/// Resolve a credential: first matching rung in ladder order wins.
pub fn resolve_credential(ctx: &LadderCtx) -> Option<Credential> {
    let rungs = [
        (
            LadderLevel::Runtime,
            &ctx.runtime_override,
            CredKind::ApiKey,
        ),
        (LadderLevel::Config, &ctx.config_key, CredKind::ApiKey),
        (LadderLevel::OAuth, &ctx.oauth_token, CredKind::BearerToken),
        (LadderLevel::LoginKey, &ctx.login_key, CredKind::ApiKey),
        (LadderLevel::Env, &ctx.env_key, CredKind::ApiKey),
        (LadderLevel::Stored, &ctx.stored_key, CredKind::ApiKey),
        (LadderLevel::Fallback, &ctx.fallback_key, CredKind::ApiKey),
    ];
    for (level, v, kind) in rungs {
        if let Some(access) = v.clone() {
            return Some(Credential {
                access,
                kind,
                level,
            });
        }
    }
    None
}

/// Parse a minimal `.env` file: `KEY=value` lines, quotes stripped, NUL
/// bytes dropped, existing keys in `out` are never overwritten
/// (process-env precedence).
pub fn parse_env_file(content: &str, out: &mut std::collections::HashMap<SmolStr, SmolStr>) {
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key: SmolStr = key.trim().to_owned().into();
        if !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') || key.is_empty() {
            continue;
        }
        let value = value.replace('\0', "");
        let value = value.trim();
        let value = value
            .strip_prefix('"')
            .and_then(|v| v.strip_suffix('"'))
            .or_else(|| value.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')))
            .unwrap_or(value);
        out.entry(key).or_insert_with(|| value.to_owned().into());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> LadderCtx {
        LadderCtx {
            runtime_override: None,
            config_key: None,
            oauth_token: None,
            login_key: None,
            env_key: None,
            stored_key: None,
            fallback_key: None,
        }
    }

    /// Pairwise priority: with only rungs i and j set, the lower rung number
    /// must win.
    #[test]
    fn ladder_priority_pairwise() {
        let levels: [(LadderLevel, fn(&mut LadderCtx, SmolStr)); 7] = [
            (LadderLevel::Runtime, |c, v| c.runtime_override = Some(v)),
            (LadderLevel::Config, |c, v| c.config_key = Some(v)),
            (LadderLevel::OAuth, |c, v| c.oauth_token = Some(v)),
            (LadderLevel::LoginKey, |c, v| c.login_key = Some(v)),
            (LadderLevel::Env, |c, v| c.env_key = Some(v)),
            (LadderLevel::Stored, |c, v| c.stored_key = Some(v)),
            (LadderLevel::Fallback, |c, v| c.fallback_key = Some(v)),
        ];
        for (i, (li, set_i)) in levels.iter().enumerate() {
            for (j, (lj, set_j)) in levels.iter().enumerate() {
                if i == j {
                    continue;
                }
                let mut c = ctx();
                set_i(&mut c, format!("key-{i:?}").into());
                set_j(&mut c, format!("key-{j:?}").into());
                let cred = resolve_credential(&c).expect("pair must resolve");
                let expected = if li < lj { *li } else { *lj };
                assert_eq!(cred.level, expected, "rung {li:?} vs {lj:?}");
            }
        }
    }

    #[test]
    fn runtime_beats_everything_single_check() {
        let mut c = ctx();
        c.runtime_override = Some("rt".into());
        c.config_key = Some("cfg".into());
        c.oauth_token = Some("oauth".into());
        c.env_key = Some("env".into());
        c.fallback_key = Some("fb".into());
        let cred = resolve_credential(&c).expect("resolve");
        assert_eq!(cred.access, "rt");
        assert_eq!(cred.level, LadderLevel::Runtime);
        assert_eq!(cred.kind, CredKind::ApiKey);
    }

    #[test]
    fn oauth_is_bearer_kind() {
        let mut c = ctx();
        c.oauth_token = Some("tok".into());
        let cred = resolve_credential(&c).expect("resolve");
        assert_eq!(cred.kind, CredKind::BearerToken);
    }

    #[test]
    fn empty_ladder_resolves_none() {
        assert!(resolve_credential(&ctx()).is_none());
    }

    /// Compile-time check of the public API: `Credential` exposes only
    /// access material — no refresh field can exist here.
    #[test]
    fn credential_has_no_refresh_surface() {
        let cred = Credential {
            access: "a".into(),
            kind: CredKind::ApiKey,
            level: LadderLevel::Env,
        };
        // Destructure exhaustively: any added field would break this.
        let Credential {
            access,
            kind: _,
            level: _,
        } = cred;
        assert_eq!(access, "a");
    }

    #[test]
    fn env_file_parsing_minimal() {
        let mut map = std::collections::HashMap::new();
        map.insert("PRE".to_owned().into(), "kept".into());
        parse_env_file(
            "# comment\nFOO=bar\nQUOTED=\"double v\"\nSQ='single'\nEMPTY=\nBAD KEY=x\nNUL=a\0b\nPRE=overwritten\n",
            &mut map,
        );
        assert_eq!(map.get("FOO").map(|s| s.as_str()), Some("bar"));
        assert_eq!(map.get("QUOTED").map(|s| s.as_str()), Some("double v"));
        assert_eq!(map.get("SQ").map(|s| s.as_str()), Some("single"));
        assert_eq!(map.get("EMPTY").map(|s| s.as_str()), Some(""));
        assert!(!map.contains_key("BAD KEY"));
        assert_eq!(map.get("NUL").map(|s| s.as_str()), Some("ab"));
        // Existing key not overwritten.
        assert_eq!(map.get("PRE").map(|s| s.as_str()), Some("kept"));
    }
}
