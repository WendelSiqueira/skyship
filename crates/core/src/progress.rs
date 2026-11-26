//! Eventos de progresso. O core nunca escreve no terminal: quem usa (CLI, GUI)
//! decide o que fazer com cada evento.

use serde::Serialize;
use std::sync::Arc;

/// Etapas visíveis para o usuário, na ordem em que a interface as mostra.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    /// Construir a imagem localmente
    Build,
    /// Garantir o serviço/instância (criar se não existir)
    Service,
    /// Enviar a imagem para o registry
    Upload,
    /// Publicar a nova versão
    Publish,
    /// Esperar o app responder
    WaitLive,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    StageStarted { stage: Stage },
    StageDone { stage: Stage },
    /// Linha de log técnico (saída do docker, detalhes da AWS)
    Log { line: String },
    /// Fim do deploy com sucesso
    Done { url: Option<String> },
    /// O deploy falhou (quem chama emite isto a partir do `Err` de [`crate::deploy`])
    Failed { message: String },
    /// O usuário cancelou o deploy
    Cancelled,
}

/// Receptor de eventos; barato de clonar e seguro entre threads.
#[derive(Clone)]
pub struct Progress(Arc<dyn Fn(Event) + Send + Sync>);

impl Progress {
    pub fn new(f: impl Fn(Event) + Send + Sync + 'static) -> Self {
        Self(Arc::new(f))
    }

    /// Descarta tudo (útil em testes).
    pub fn silent() -> Self {
        Self::new(|_| {})
    }

    pub fn emit(&self, e: Event) {
        (self.0)(e)
    }

    pub fn start(&self, stage: Stage) {
        self.emit(Event::StageStarted { stage });
    }

    pub fn done(&self, stage: Stage) {
        self.emit(Event::StageDone { stage });
    }

    pub fn log(&self, line: impl Into<String>) {
        self.emit(Event::Log { line: line.into() });
    }
}
