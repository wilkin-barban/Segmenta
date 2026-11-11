//! Locale detection and message catalogs for Segmenta.
//!
//! The protocol never carries translated text: JSON-RPC keeps stable numeric `code`s and
//! the clients resolve the human text here. Catalogs are exhaustive `match`es, so the
//! compiler itself enforces parity between `en`, `es` and `pt-BR`.

use std::sync::OnceLock;

/// Supported UI languages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Locale {
    /// English, the fallback.
    En,
    /// Spanish.
    Es,
    /// Brazilian Portuguese.
    PtBr,
}

impl Locale {
    /// Stable BCP-47-ish tag (`en`, `es`, `pt-BR`).
    pub fn code(self) -> &'static str {
        match self {
            Locale::En => "en",
            Locale::Es => "es",
            Locale::PtBr => "pt-BR",
        }
    }

    /// Normalize a locale tag (`es_ES.UTF-8` → `Es`, `pt_BR` → `PtBr`).
    pub fn from_tag(tag: &str) -> Locale {
        let tag = tag.trim();
        if tag.is_empty() {
            return Locale::En;
        }
        let lowered = tag.to_ascii_lowercase().replace('_', "-");
        let language = lowered.split(['.', '@', '-']).next().unwrap_or_default();
        match language {
            "es" => Locale::Es,
            "pt" => Locale::PtBr,
            _ => Locale::En,
        }
    }

    /// Detection order of the QA: `LC_ALL`, `LC_MESSAGES`, `LANG`, then `en`.
    pub fn detect() -> Locale {
        detect_from(|name| std::env::var(name).ok())
    }
}

/// Locale cached for the process lifetime (the UI does not switch language at runtime).
pub fn current() -> Locale {
    static LOCALE: OnceLock<Locale> = OnceLock::new();
    *LOCALE.get_or_init(Locale::detect)
}

/// Detection over an injectable environment (tests never mutate the process env).
pub fn detect_from(env: impl Fn(&str) -> Option<String>) -> Locale {
    // `C`/`POSIX` mean "no localization": fall through to the next variable, then `en`.
    let candidates = ["LC_ALL", "LC_MESSAGES", "LANG"];
    for name in candidates {
        if let Some(value) = env(name) {
            let value = value.trim();
            if value.is_empty() || value == "C" || value == "POSIX" {
                continue;
            }
            let locale = Locale::from_tag(value);
            if locale != Locale::En || value.to_ascii_lowercase().starts_with("en") {
                return locale;
            }
            // An unknown language is a real answer only when it is explicit: keep
            // looking, then fall back to English.
            return Locale::En;
        }
    }
    Locale::En
}

macro_rules! catalog {
    ($( $variant:ident => { en: $en:expr, es: $es:expr, pt: $pt:expr } ),+ $(,)?) => {
        /// Every visible UI/CLI string.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum Msg {
            $(
                #[doc = $en]
                $variant,
            )+
        }

        impl Msg {
            /// Every message, for parity tests.
            pub const ALL: &'static [Msg] = &[$(Msg::$variant),+];
        }

        /// Translate one message.
        pub fn tr(locale: Locale, msg: Msg) -> &'static str {
            match msg {
                $(
                    Msg::$variant => match locale {
                        Locale::En => $en,
                        Locale::Es => $es,
                        Locale::PtBr => $pt,
                    },
                )+
            }
        }
    };
}

catalog! {
    AppTitle => { en: "Segmenta", es: "Segmenta", pt: "Segmenta" },
    WindowTitle => { en: "Downloads", es: "Descargas", pt: "Downloads" },
    AddUrl => { en: "Add URL", es: "Añadir URL", pt: "Adicionar URL" },
    FilterAll => { en: "All", es: "Todas", pt: "Todas" },
    FilterActive => { en: "Active", es: "Activas", pt: "Ativas" },
    FilterPaused => { en: "Paused", es: "Pausadas", pt: "Pausadas" },
    FilterCompleted => { en: "Completed", es: "Completadas", pt: "Concluídas" },
    FilterFailed => { en: "Failed", es: "Fallidas", pt: "Com falha" },
    FilterCancelled => { en: "Cancelled", es: "Canceladas", pt: "Canceladas" },
    Queues => { en: "Queues", es: "Colas", pt: "Filas" },
    Loading => { en: "Connecting to the daemon…", es: "Conectando con el daemon…", pt: "Conectando ao daemon…" },
    EmptyTitle => { en: "No downloads yet", es: "Todavía no hay descargas", pt: "Ainda não há downloads" },
    EmptyBody => { en: "Add a URL to start downloading.", es: "Añadí una URL para empezar a descargar.", pt: "Adicione uma URL para começar a baixar." },
    ErrorTitle => { en: "The daemon is not reachable", es: "No se puede contactar con el daemon", pt: "Não é possível contatar o daemon" },
    ErrorBody => { en: "Start the socket and try again:", es: "Iniciá el socket e intentá de nuevo:", pt: "Inicie o socket e tente de novo:" },
    ErrorCommand => { en: "systemctl --user enable --now segmenta.socket", es: "systemctl --user enable --now segmenta.socket", pt: "systemctl --user enable --now segmenta.socket" },
    Retry => { en: "Retry", es: "Reintentar", pt: "Tentar de novo" },
    Reload => { en: "Reload", es: "Recargar", pt: "Recarregar" },
    Preferences => { en: "Preferences", es: "Preferencias", pt: "Preferências" },
    About => { en: "About", es: "Acerca de", pt: "Sobre" },
    Quit => { en: "Quit", es: "Salir", pt: "Sair" },
    Search => { en: "Search", es: "Buscar", pt: "Pesquisar" },
    Pause => { en: "Pause", es: "Pausar", pt: "Pausar" },
    Resume => { en: "Resume", es: "Reanudar", pt: "Retomar" },
    Cancel => { en: "Cancel", es: "Cancelar", pt: "Cancelar" },
    Delete => { en: "Delete", es: "Eliminar", pt: "Excluir" },
    OpenFolder => { en: "Open folder", es: "Abrir carpeta", pt: "Abrir pasta" },
    Start => { en: "Add", es: "Añadir", pt: "Adicionar" },
    StartNow => { en: "Start now", es: "Comenzar ahora", pt: "Começar agora" },
    Destination => { en: "Destination folder", es: "Carpeta de destino", pt: "Pasta de destino" },
    ChooseFolder => { en: "Choose…", es: "Elegir…", pt: "Escolher…" },
    Filename => { en: "File name", es: "Nombre del archivo", pt: "Nome do arquivo" },
    Connections => { en: "Connections", es: "Conexiones", pt: "Conexões" },
    UrlLabel => { en: "URL", es: "URL", pt: "URL" },
    UrlRequired => { en: "Enter an http:// or https:// URL", es: "Ingresá una URL http:// o https://", pt: "Informe uma URL http:// ou https://" },
    FolderRequired => { en: "Choose a destination folder", es: "Elegí una carpeta de destino", pt: "Escolha uma pasta de destino" },
    AddFailed => { en: "Could not add the download", es: "No se pudo añadir la descarga", pt: "Não foi possível adicionar o download" },
    Added => { en: "Download added", es: "Descarga añadida", pt: "Download adicionado" },
    Paused => { en: "Paused", es: "Pausada", pt: "Pausado" },
    Resumed => { en: "Resumed", es: "Reanudada", pt: "Retomado" },
    Cancelled => { en: "Cancelled", es: "Cancelada", pt: "Cancelado" },
    Deleted => { en: "Deleted", es: "Eliminada", pt: "Excluído" },
    ConfirmCancelTitle => { en: "Cancel this download?", es: "¿Cancelar esta descarga?", pt: "Cancelar este download?" },
    ConfirmCancelBody => { en: "The partial file is kept.", es: "El archivo parcial se conserva.", pt: "O arquivo parcial é mantido." },
    ConfirmDeleteTitle => { en: "Delete this task?", es: "¿Eliminar esta tarea?", pt: "Excluir esta tarefa?" },
    ConfirmDeleteBody => { en: "The task is removed from the list. The file is not touched.", es: "La tarea se elimina de la lista. El archivo no se toca.", pt: "A tarefa é removida da lista. O arquivo não é tocado." },
    Confirm => { en: "Confirm", es: "Confirmar", pt: "Confirmar" },
    Settings => { en: "Settings", es: "Configuración", pt: "Configurações" },
    DownloadFolder => { en: "Download folder", es: "Carpeta de descargas", pt: "Pasta de downloads" },
    SpeedLimit => { en: "Global speed limit (KiB/s, 0 = unlimited)", es: "Límite de velocidad global (KiB/s, 0 = ilimitado)", pt: "Limite global de velocidade (KiB/s, 0 = ilimitado)" },
    Autostart => { en: "Start with the session", es: "Iniciar con la sesión", pt: "Iniciar com a sessão" },
    Appearance => { en: "Appearance", es: "Apariencia", pt: "Aparência" },
    AppearanceSystem => { en: "Follow the system", es: "Seguir al sistema", pt: "Seguir o sistema" },
    AppearanceLight => { en: "Light", es: "Claro", pt: "Claro" },
    AppearanceDark => { en: "Dark", es: "Oscuro", pt: "Escuro" },
    MaxRetries => { en: "Retries", es: "Reintentos", pt: "Tentativas" },
    AllowLocal => { en: "Allow local addresses", es: "Permitir direcciones locales", pt: "Permitir endereços locais" },
    Saved => { en: "Saved", es: "Guardado", pt: "Salvo" },
    SaveFailed => { en: "Could not save", es: "No se pudo guardar", pt: "Não foi possível salvar" },
    CategoriesReadOnly => { en: "Per-category folders (read only)", es: "Carpetas por categoría (sólo lectura)", pt: "Pastas por categoria (somente leitura)" },
    SecretBackend => { en: "Secret backend", es: "Backend de secretos", pt: "Backend de segredos" },
    DaemonVersion => { en: "Daemon", es: "Daemon", pt: "Daemon" },
    CompletedNotification => { en: "Download finished", es: "Descarga terminada", pt: "Download concluído" },
    FailedNotification => { en: "Download failed", es: "Descarga fallida", pt: "Download falhou" },
    Speed => { en: "Speed", es: "Velocidad", pt: "Velocidade" },
    Eta => { en: "Remaining", es: "Restante", pt: "Restante" },
    UnknownSize => { en: "unknown", es: "desconocido", pt: "desconhecido" },
    Segments => { en: "Segments", es: "Segmentos", pt: "Segmentos" },
    NoResults => { en: "No downloads match the filter", es: "Ninguna descarga coincide con el filtro", pt: "Nenhum download corresponde ao filtro" },
    ClearSearch => { en: "Clear search", es: "Limpiar búsqueda", pt: "Limpar pesquisa" },
    ToastAdded => { en: "Added: {filename}", es: "Añadida: {filename}", pt: "Adicionado: {filename}" },
    MenuMain => { en: "Main menu", es: "Menú principal", pt: "Menu principal" },
    MenuAddTooltip => { en: "Add a download (Ctrl+N)", es: "Añadir una descarga (Ctrl+N)", pt: "Adicionar um download (Ctrl+N)" },
    InvalidListResponse => { en: "The daemon answered an invalid list: {error}", es: "Respuesta inválida del daemon al listar: {error}", pt: "Resposta inválida do daemon ao listar: {error}" },
    ConfirmCancelBodyLong => { en: "The transfer stops and the task is marked cancelled.", es: "Se detendrá la transferencia y la tarea quedará como cancelada.", pt: "A transferência para e a tarefa fica cancelada." },
    ConfirmDeleteBodyLong => { en: "The task record is removed. The downloaded file is not touched.", es: "Se borrará el registro de la tarea. El archivo descargado no se toca.", pt: "O registro da tarefa é removido. O arquivo baixado não é tocado." },
    StatusPending => { en: "Waiting", es: "En espera", pt: "Aguardando" },
    StatusProbing => { en: "Probing", es: "Analizando", pt: "Analisando" },
    StatusDownloading => { en: "Downloading", es: "Descargando", pt: "Baixando" },
    StatusPaused => { en: "Paused", es: "Pausada", pt: "Pausado" },
    StatusVerifying => { en: "Verifying", es: "Verificando", pt: "Verificando" },
    StatusCompleted => { en: "Completed", es: "Completada", pt: "Concluído" },
    StatusFailed => { en: "Failed", es: "Fallida", pt: "Com falha" },
    StatusCancelled => { en: "Cancelled", es: "Cancelada", pt: "Cancelado" },
    StatusUnknown => { en: "Unknown", es: "Desconocida", pt: "Desconhecido" },
    // CLI
    CliAdded => { en: "added", es: "añadida", pt: "adicionado" },
    CliFile => { en: "file", es: "archivo", pt: "arquivo" },
    CliStatus => { en: "status", es: "estado", pt: "status" },
    CliTableId => { en: "ID", es: "ID", pt: "ID" },
    CliTableStatus => { en: "STATUS", es: "ESTADO", pt: "STATUS" },
    CliTableProgress => { en: "PROG", es: "PROG", pt: "PROG" },
    CliTableSize => { en: "SIZE", es: "TAMAÑO", pt: "TAMANHO" },
    CliTableSpeed => { en: "SPEED", es: "VELOCIDAD", pt: "VELOCIDADE" },
    CliTableEta => { en: "ETA", es: "RESTANTE", pt: "RESTANTE" },
    CliTableName => { en: "NAME", es: "NOMBRE", pt: "NOME" },
    CliNoDownloads => { en: "No downloads.", es: "No hay descargas.", pt: "Não há downloads." },
    CliNoQueues => { en: "No queues.", es: "No hay colas.", pt: "Não há filas." },
    CliPaused => { en: "paused", es: "pausada", pt: "pausado" },
    CliResumed => { en: "resumed", es: "reanudada", pt: "retomado" },
    CliCancelled => { en: "cancelled", es: "cancelada", pt: "cancelado" },
    CliDeleted => { en: "deleted", es: "eliminada", pt: "excluído" },
    CliDaemonStopped => { en: "daemon stopped", es: "daemon detenido", pt: "daemon parado" },
    CliQueueCreated => { en: "created queue", es: "cola creada", pt: "fila criada" },
    CliQueueRemoved => { en: "removed queue", es: "cola eliminada", pt: "fila removida" },
    CliHello => { en: "segmenta daemon", es: "daemon de segmenta", pt: "daemon do segmenta" },
    CliProtocol => { en: "protocol", es: "protocolo", pt: "protocolo" },
    CliUptime => { en: "uptime", es: "activo", pt: "ativo" },
    CliMemory => { en: "memory", es: "memoria", pt: "memória" },
    CliConnections => { en: "connections", es: "conexiones", pt: "conexões" },
    CliSecrets => { en: "secrets", es: "secretos", pt: "segredos" },
    CliPairUsage => { en: "expects KEY VALUE pairs", es: "espera pares CLAVE VALOR", pt: "espera pares CHAVE VALOR" },
    CliHeaderUsage => { en: "header must be \"Name: value\"", es: "la cabecera debe ser \"Nombre: valor\"", pt: "o cabeçalho deve ser \"Nome: valor\"" },
    CliShortIdUnknown => { en: "no task starting with", es: "no hay tarea que empiece con", pt: "não há tarefa começando com" },
    CliShortIdAmbiguous => { en: "matches several tasks; use a longer prefix", es: "coincide con varias tareas; usá un prefijo más largo", pt: "corresponde a várias tarefas; use um prefixo maior" },
}

/// Localized text for a stable JSON-RPC error code.
///
/// The daemon never translates: the client resolves the code and falls back to the
/// daemon's message when the code is unknown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ErrorText {
    /// 1001
    TaskNotFound,
    /// 1002
    TaskAlreadyActive,
    /// 1003
    TaskNotResumable,
    /// 1004
    InvalidUrl,
    /// 1005
    DestinationUnwritable,
    /// 1006
    DiskFull,
    /// 1007
    BlockedPrivateIp,
    /// 1008
    ChecksumMismatch,
    /// 1009
    PayloadTooLarge,
    /// 1010
    UnauthorizedPeer,
    /// 1011
    TaskNotPausable,
    /// 1021
    TooManyRedirects,
    /// 1022
    ResourceGone,
    /// 1023
    AuthRequired,
    /// 1024
    ProxyAuth,
    /// 1025
    RangeOutOfBounds,
    /// 1026
    SizeMismatch,
    /// 1027
    TruncatedResponse,
    /// 1028
    Unsupported,
    /// 1029
    Network,
    /// 1030
    Cancelled,
    /// 1031
    KeyringUnavailable,
    /// 1032
    MalformedHeader,
    /// Daemon/internal failure (`-32603`, connection problems).
    Internal,
}

impl ErrorText {
    /// Localized description of a stable error code.
    pub fn from_code(code: i64) -> Option<ErrorText> {
        Some(match code {
            1001 => ErrorText::TaskNotFound,
            1002 => ErrorText::TaskAlreadyActive,
            1003 => ErrorText::TaskNotResumable,
            1004 => ErrorText::InvalidUrl,
            1005 => ErrorText::DestinationUnwritable,
            1006 => ErrorText::DiskFull,
            1007 => ErrorText::BlockedPrivateIp,
            1008 => ErrorText::ChecksumMismatch,
            1009 => ErrorText::PayloadTooLarge,
            1010 => ErrorText::UnauthorizedPeer,
            1011 => ErrorText::TaskNotPausable,
            1021 => ErrorText::TooManyRedirects,
            1022 => ErrorText::ResourceGone,
            1023 => ErrorText::AuthRequired,
            1024 => ErrorText::ProxyAuth,
            1025 => ErrorText::RangeOutOfBounds,
            1026 => ErrorText::SizeMismatch,
            1027 => ErrorText::TruncatedResponse,
            1028 => ErrorText::Unsupported,
            1029 => ErrorText::Network,
            1030 => ErrorText::Cancelled,
            1031 => ErrorText::KeyringUnavailable,
            1032 => ErrorText::MalformedHeader,
            -32603 | -32600 | -32601 | -32602 | -32700 => ErrorText::Internal,
            _ => return None,
        })
    }

    /// Every known code, for parity tests.
    pub const ALL: &'static [ErrorText] = &[
        ErrorText::TaskNotFound,
        ErrorText::TaskAlreadyActive,
        ErrorText::TaskNotResumable,
        ErrorText::InvalidUrl,
        ErrorText::DestinationUnwritable,
        ErrorText::DiskFull,
        ErrorText::BlockedPrivateIp,
        ErrorText::ChecksumMismatch,
        ErrorText::PayloadTooLarge,
        ErrorText::UnauthorizedPeer,
        ErrorText::TaskNotPausable,
        ErrorText::TooManyRedirects,
        ErrorText::ResourceGone,
        ErrorText::AuthRequired,
        ErrorText::ProxyAuth,
        ErrorText::RangeOutOfBounds,
        ErrorText::SizeMismatch,
        ErrorText::TruncatedResponse,
        ErrorText::Unsupported,
        ErrorText::Network,
        ErrorText::Cancelled,
        ErrorText::KeyringUnavailable,
        ErrorText::MalformedHeader,
        ErrorText::Internal,
    ];
}

/// Translate one error code, falling back to `None` for unknown codes.
pub fn tr_error(locale: Locale, error: ErrorText) -> &'static str {
    use ErrorText::*;
    match error {
        TaskNotFound => match locale {
            Locale::En => "The task does not exist",
            Locale::Es => "La tarea no existe",
            Locale::PtBr => "A tarefa não existe",
        },
        TaskAlreadyActive => match locale {
            Locale::En => "The task is already running",
            Locale::Es => "La tarea ya está en marcha",
            Locale::PtBr => "A tarefa já está em execução",
        },
        TaskNotResumable => match locale {
            Locale::En => "This task cannot be resumed",
            Locale::Es => "Esta tarea no se puede reanudar",
            Locale::PtBr => "Esta tarefa não pode ser retomada",
        },
        TaskNotPausable => match locale {
            Locale::En => "This task is not running and cannot be paused",
            Locale::Es => "Esta tarea no está en marcha y no se puede pausar",
            Locale::PtBr => "Esta tarefa não está em execução e não pode ser pausada",
        },
        InvalidUrl => match locale {
            Locale::En => "The URL is not a valid http(s) address",
            Locale::Es => "La URL no es una dirección http(s) válida",
            Locale::PtBr => "A URL não é um endereço http(s) válido",
        },
        DestinationUnwritable => match locale {
            Locale::En => "The destination folder is not writable",
            Locale::Es => "La carpeta de destino no tiene permisos de escritura",
            Locale::PtBr => "A pasta de destino não tem permissão de escrita",
        },
        DiskFull => match locale {
            Locale::En => "Not enough disk space",
            Locale::Es => "No hay espacio en disco",
            Locale::PtBr => "Não há espaço em disco",
        },
        BlockedPrivateIp => match locale {
            Locale::En => "The URL points to a private address (blocked for safety)",
            Locale::Es => "La URL apunta a una dirección privada (bloqueada por seguridad)",
            Locale::PtBr => "A URL aponta para um endereço privado (bloqueado por segurança)",
        },
        ChecksumMismatch => match locale {
            Locale::En => "The file checksum does not match",
            Locale::Es => "El checksum del archivo no coincide",
            Locale::PtBr => "O checksum do arquivo não confere",
        },
        PayloadTooLarge => match locale {
            Locale::En => "The request is too large",
            Locale::Es => "La petición es demasiado grande",
            Locale::PtBr => "A requisição é grande demais",
        },
        UnauthorizedPeer => match locale {
            Locale::En => "The daemon rejected this user",
            Locale::Es => "El daemon rechazó a este usuario",
            Locale::PtBr => "O daemon rejeitou este usuário",
        },
        TooManyRedirects => match locale {
            Locale::En => "Too many redirects",
            Locale::Es => "Demasiadas redirecciones",
            Locale::PtBr => "Redirecionamentos demais",
        },
        ResourceGone => match locale {
            Locale::En => "The file no longer exists on the server",
            Locale::Es => "El archivo ya no existe en el servidor",
            Locale::PtBr => "O arquivo não existe mais no servidor",
        },
        AuthRequired => match locale {
            Locale::En => "The server requires credentials",
            Locale::Es => "El servidor requiere credenciales",
            Locale::PtBr => "O servidor exige credenciais",
        },
        ProxyAuth => match locale {
            Locale::En => "The proxy requires credentials",
            Locale::Es => "El proxy requiere credenciales",
            Locale::PtBr => "O proxy exige credenciais",
        },
        RangeOutOfBounds => match locale {
            Locale::En => "The server rejected the byte range",
            Locale::Es => "El servidor rechazó el rango de bytes",
            Locale::PtBr => "O servidor rejeitou o intervalo de bytes",
        },
        SizeMismatch => match locale {
            Locale::En => "The downloaded size does not match the server",
            Locale::Es => "El tamaño descargado no coincide con el del servidor",
            Locale::PtBr => "O tamanho baixado não confere com o do servidor",
        },
        TruncatedResponse => match locale {
            Locale::En => "The connection closed before the file ended",
            Locale::Es => "La conexión se cortó antes de terminar el archivo",
            Locale::PtBr => "A conexão caiu antes de terminar o arquivo",
        },
        Unsupported => match locale {
            Locale::En => "The server answered in a way Segmenta cannot use",
            Locale::Es => "El servidor respondió de una forma que Segmenta no puede usar",
            Locale::PtBr => "O servidor respondeu de um jeito que o Segmenta não pode usar",
        },
        Network => match locale {
            Locale::En => "Network error",
            Locale::Es => "Error de red",
            Locale::PtBr => "Erro de rede",
        },
        Cancelled => match locale {
            Locale::En => "Cancelled",
            Locale::Es => "Cancelada",
            Locale::PtBr => "Cancelado",
        },
        KeyringUnavailable => match locale {
            Locale::En => "No keyring available: install libsecret-tools and unlock the keyring",
            Locale::Es => {
                "No hay llavero disponible: instalá libsecret-tools y desbloqueá el llavero"
            }
            Locale::PtBr => {
                "Não há chaveiro disponível: instale libsecret-tools e desbloqueie o chaveiro"
            }
        },
        MalformedHeader => match locale {
            Locale::En => "The server sent a malformed header",
            Locale::Es => "El servidor envió una cabecera malformada",
            Locale::PtBr => "O servidor enviou um cabeçalho malformado",
        },
        Internal => match locale {
            Locale::En => "Internal error",
            Locale::Es => "Error interno",
            Locale::PtBr => "Erro interno",
        },
    }
}
