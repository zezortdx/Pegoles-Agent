//! The native disclosure shown before experimental team mode is turned
//! on. Static copy in the saved locale (en or pt-BR); no task, guest or
//! model text ever enters it.

use crate::input_consent::InputPrompt;
use crate::preferences::PreferenceStore;

pub(crate) fn prompt(portuguese: bool) -> InputPrompt {
    if portuguese {
        InputPrompt {
            title: "Ativar o modo equipe (experimental)?",
            message: "Cada tarefa passará por uma equipe de funções na nuvem da Anthropic: capitão (plano), pesquisador, executor, crítico e capitão (resposta final). Isso envia mais pedidos por tarefa, com o texto da tarefa, capturas da tela do computador virtual do Pegoles (nunca a tela do seu Mac) e as anotações da própria equipe. Usa mais da sua cota de API e é mais lento. Somente o executor age, e cada ação continua passando pela política de segurança, pelas aprovações e pela retomada de controle. O Pegoles Local nunca usa o modo equipe.\n\nContinue só se você ativou o modo equipe nas Configurações agora.".into(),
            confirm: "Ativar modo equipe",
            cancel: "Cancelar",
        }
    } else {
        InputPrompt {
            title: "Turn on team mode (experimental)?",
            message: "Each task will go through a team of roles on Anthropic’s cloud: captain (plan), researcher, executor, critic and captain (final answer). That sends more requests per task, including the task text, screenshots of Pegoles’ virtual computer (never your Mac’s screen) and the team’s own notes. It uses more of your API quota and is slower. Only the executor acts, and every action still goes through the safety policy, approvals and takeover. Pegoles Local never uses team mode.\n\nOnly continue if you turned on team mode in Settings just now.".into(),
            confirm: "Turn on team mode",
            cancel: "Cancel",
        }
    }
}

/// Ask with the saved locale. A preference file that cannot be read means
/// no question and no consent.
pub(crate) fn confirm_with_store(
    store: &PreferenceStore,
    decide: impl FnOnce(InputPrompt) -> Result<bool, String>,
) -> Result<bool, String> {
    let preferences = store.read()?;
    decide(prompt(preferences.locale == "pt-BR"))
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
                    .join("../../../target/review/team-planner-tests")
                    .join(format!("team-consent-{}", pegoles_protocol::TaskId::new())),
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
    fn the_disclosure_names_the_cloud_the_extra_data_and_the_unchanged_safety() {
        let en = prompt(false);
        for needle in [
            "Anthropic",
            "screenshots",
            "never your Mac",
            "approvals",
            "Pegoles Local never",
        ] {
            assert!(en.message.contains(needle), "{needle}");
        }
        let pt = prompt(true);
        for needle in [
            "Anthropic",
            "capturas",
            "nunca a tela do seu Mac",
            "aprovações",
            "Pegoles Local nunca",
        ] {
            assert!(pt.message.contains(needle), "{needle}");
        }
        assert_ne!(en.confirm, en.cancel);
        assert_ne!(pt.confirm, pt.cancel);
    }

    #[test]
    fn the_saved_locale_picks_the_language_and_a_corrupt_one_asks_nothing() {
        let f = Fixture::new();
        f.locale("pt-BR");
        assert!(!confirm_with_store(&f.store(), |prompt| {
            assert_eq!(prompt.cancel, "Cancelar");
            Ok(false)
        })
        .unwrap());
        let mut invalid = serde_json::to_value(Preferences::default()).unwrap();
        invalid["locale"] = "<script>".into();
        std::fs::write(
            f.0.join("profile.json"),
            serde_json::to_vec(&invalid).unwrap(),
        )
        .unwrap();
        assert!(confirm_with_store(&f.store(), |_| panic!("invalid locale prompted")).is_err());
    }
}
