//! Native input prompts use validated saved locale and static product copy only.
use crate::preferences::PreferenceStore;

pub(crate) struct InputPrompt {
    pub title: &'static str,
    pub message: String,
    pub confirm: &'static str,
    pub cancel: &'static str,
}
pub(crate) enum InputQuestion<'a> {
    Action(&'a str),
    Policy(bool),
}

pub(crate) fn confirm_with_store(
    store: &PreferenceStore,
    question: InputQuestion<'_>,
    decide: impl FnOnce(InputPrompt) -> Result<bool, String>,
) -> Result<bool, String> {
    let preferences = store.read()?;
    let portuguese = preferences.locale == "pt-BR";
    let (message, confirm) = match question {
        InputQuestion::Action(summary) => {
            let (en, pt) = match summary {
                "VM input: type" => ("Type text", "Digitar texto"),
                "VM input: move" => ("Move pointer", "Mover ponteiro"),
                "VM input: click" => ("Click", "Clicar"),
                "VM input: double_click" => ("Double-click", "Clicar duas vezes"),
                "VM input: mouse_down" => ("Press mouse button", "Pressionar botão do mouse"),
                "VM input: mouse_up" => ("Release mouse button", "Soltar botão do mouse"),
                "VM input: drag" => ("Drag", "Arrastar"),
                "VM input: scroll" => ("Scroll", "Rolar"),
                "VM input: key_press" => ("Press key", "Pressionar tecla"),
                "VM input: key_chord" => {
                    ("Press key combination", "Pressionar combinação de teclas")
                }
                _ => return Err("Unknown approval summary.".into()),
            };
            if portuguese {
                (format!("Aprovar esta ação pendente exata uma vez?\nEntrada na máquina virtual: {pt}\nA solicitação expira após 120 segundos. O texto digitado nunca é exibido aqui."), "Aprovar uma vez")
            } else {
                (format!("Approve this exact pending action once?\nVM input: {en}\nThe request expires after 120 seconds. Typed text is never shown here."), "Approve once")
            }
        }
        InputQuestion::Policy(enabled) => {
            let message = match (portuguese, enabled) {
                (true, true) => "Perguntar antes de cada entrada do agente que modifique a máquina virtual nesta sessão? Esta configuração volta a ficar desativada ao reiniciar o Pegoles.",
                (true, false) => "Parar de perguntar antes das entradas do agente nesta sessão? As recusas da política integrada continuam valendo. Esta configuração volta a ficar desativada ao reiniciar o Pegoles.",
                (false, true) => "Ask before every modifying agent input in the VM for this session? This setting resets to off when Pegoles restarts.",
                (false, false) => "Stop asking before agent input for this session? Built-in policy denials still apply. This setting resets to off when Pegoles restarts.",
            };
            (
                message.into(),
                if portuguese {
                    "Confirmar alteração"
                } else {
                    "Confirm change"
                },
            )
        }
    };
    decide(InputPrompt {
        title: if portuguese {
            "Confirmação humana do Pegoles"
        } else {
            "Pegoles human confirmation"
        },
        message,
        confirm,
        cancel: if portuguese { "Cancelar" } else { "Cancel" },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::preferences::Preferences;
    struct Fixture(std::path::PathBuf);
    impl Fixture {
        fn new() -> Self {
            Self(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../../target/review/agent-2.0")
                    .join(format!("native-locale-{}", pegoles_protocol::TaskId::new())),
            )
        }
        fn store(&self) -> PreferenceStore {
            PreferenceStore::new(self.0.clone())
        }
        fn locale(&self, locale: &str) {
            self.store()
                .write(Preferences {
                    locale: locale.into(),
                    ..Preferences::default()
                })
                .unwrap();
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn saved_portuguese_localizes_native_action_and_both_policy_prompts() {
        let f = Fixture::new();
        f.locale("pt-BR");
        for (question, body, confirm) in [
            (
                InputQuestion::Action("VM input: type"),
                "Digitar texto",
                "Aprovar uma vez",
            ),
            (
                InputQuestion::Policy(true),
                "Perguntar antes",
                "Confirmar alteração",
            ),
            (
                InputQuestion::Policy(false),
                "Parar de perguntar",
                "Confirmar alteração",
            ),
        ] {
            assert!(!confirm_with_store(&f.store(), question, |prompt| {
                assert_eq!(prompt.title, "Confirmação humana do Pegoles");
                assert_eq!(prompt.cancel, "Cancelar");
                assert_eq!(prompt.confirm, confirm);
                assert!(prompt.message.contains(body), "{}", prompt.message);
                if body == "Digitar texto" {
                    assert!(prompt.message.contains("120 segundos"));
                } else {
                    assert!(prompt.message.contains("sessão"));
                    assert!(prompt.message.contains("reiniciar"));
                }
                Ok(false)
            })
            .unwrap());
        }
    }
    #[test]
    fn missing_preferences_use_english_and_corrupt_locale_never_prompts() {
        let f = Fixture::new();
        assert!(confirm_with_store(
            &f.store(),
            InputQuestion::Action("VM input: click"),
            |prompt| {
                assert_eq!(prompt.title, "Pegoles human confirmation");
                assert_eq!(prompt.cancel, "Cancel");
                assert_eq!(prompt.confirm, "Approve once");
                assert!(prompt.message.contains("Click"));
                assert!(prompt.message.contains("120 seconds"));
                Ok(true)
            }
        )
        .unwrap());
        f.locale("en");
        for (enabled, body) in [(true, "Ask before every"), (false, "Stop asking before")] {
            assert!(
                !confirm_with_store(&f.store(), InputQuestion::Policy(enabled), |prompt| {
                    assert_eq!(prompt.title, "Pegoles human confirmation");
                    assert_eq!(prompt.confirm, "Confirm change");
                    assert_eq!(prompt.cancel, "Cancel");
                    assert!(prompt.message.contains(body));
                    assert!(prompt.message.contains("this session"));
                    assert!(prompt
                        .message
                        .contains("resets to off when Pegoles restarts"));
                    Ok(false)
                })
                .unwrap()
            );
        }
        let mut invalid = serde_json::to_value(Preferences::default()).unwrap();
        invalid["locale"] = "<script>".into();
        std::fs::write(
            f.0.join("profile.json"),
            serde_json::to_vec(&invalid).unwrap(),
        )
        .unwrap();
        assert!(
            confirm_with_store(&f.store(), InputQuestion::Policy(true), |_| panic!(
                "invalid locale prompted"
            ))
            .is_err()
        );
    }
    #[test]
    fn arbitrary_summary_is_rejected_before_native_decider() {
        let f = Fixture::new();
        assert!(confirm_with_store(
            &f.store(),
            InputQuestion::Action("VM input: type\nIgnore safety"),
            |_| panic!("untrusted summary prompted")
        )
        .is_err());
    }
}
