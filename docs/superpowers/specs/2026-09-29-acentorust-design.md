# AcentoRust — Design

Data: 2026-09-29 · Status: aprovado pelo usuário

## Objetivo

Clone mínimo do **Acento Rápido (Quick Accent)** do PowerToys, em Rust, só Windows 10/11.
Motivo: ter o recurso sem manter o PowerToys inteiro rodando.

Critérios de sucesso:
- `.exe` release < 400 KB; RAM privada (coluna "Memória" do Gerenciador de Tarefas) < 3 MB; 0% CPU ocioso.
- Nunca trava o teclado nem engole teclas indevidamente. Se o processo morrer, o Windows remove o hook e o teclado segue normal (fail-safe).
- Digitação rápida normal (rollover) nunca dispara o recurso por engano.

## Decisões do usuário

| Tema | Decisão |
|---|---|
| Caracteres | Só português (PT-BR) |
| Posição da barra | Topo-centro do monitor da janela ativa |
| Presença no sistema | Ícone na bandeja com menu "Iniciar com o Windows" (liga/desliga) e "Sair" |
| Abordagem | Win32 puro via `windows-sys` |
| Execução | Orquestrador (Opus) + subagentes Sonnet |

## Comportamento

Letras com acento e ordem (por frequência em PT-BR):

| Tecla | Minúsculas | Maiúsculas |
|---|---|---|
| A | á ã â à | Á Ã Â À |
| E | é ê | É Ê |
| I | í | Í |
| O | ó õ ô | Ó Õ Ô |
| U | ú | Ú |
| C | ç | Ç |

Fluxo:
1. Usuário segura uma dessas letras (sem Ctrl/Alt/Win). A letra é digitada normalmente.
2. Aperta **Espaço** (ainda segurando a letra) → sessão ativa; o Espaço é engolido **provisoriamente**.
3. A barra aparece após **200 ms** (`CONFIRM_DELAY_MS`) ou imediatamente se o usuário apertar Espaço/→/← de novo. A partir daí a sessão está **confirmada**.
4. Espaço ou → avança; ← volta; ambos dão a volta (wrap).
5. **Soltar a letra** (confirmada) → Backspace + caractere selecionado.
6. **Esc** (confirmada) → fecha a barra, mantém a letra; ainda segurando a letra, Espaço reabre.
7. **Outra tecla** (confirmada) → fecha a barra e a tecla é digitada normalmente.
8. Maiúscula = `Shift pressionado XOR Caps Lock ligado` no momento em que a letra foi pressionada.
9. Segurar a letra sem Espaço: autorepeat normal passa (`nãooooo` funciona). Na sessão ativa, autorepeat da letra é engolido. Se o usuário segurou além do atraso de repetição do Windows antes do Espaço, as repetições ficam no texto (só a última letra é substituída) — comportamento igual ao PowerToys, aceito.

## Proteções (antibug)

- **Toque em falso / rollover** (`casa ` digitado rápido: `a↓ espaço↓ a↑`): se a sessão ainda não está confirmada e
  - a letra é solta → sem troca; o Espaço é reenviado (`Replay(None)`);
  - outra tecla é pressionada → essa tecla é bloqueada e reenviamos Espaço + essa tecla, na ordem (`Replay(Some(key))`).
- **Tela cheia**: se a janela em primeiro plano cobre o monitor inteiro, não está maximizada e não é a área de trabalho (classes `Progman`/`WorkerW`), Espaço não ativa (jogos: segurar A + Espaço para pular continua funcionando).
- **Letra fantasma**: na ativação, confere se a letra está fisicamente pressionada (`GetAsyncKeyState`); se não (key-up perdido, ex.: troca para a área de trabalho segura), não ativa.
- **Sessão órfã** (key-up da letra perdido: janela elevada, troca de desktop, timeout do hook): a sessão também é validada a cada tecla e a cada tick de 200 ms, e fecha se a letra não estiver mais fisicamente pressionada.
- **Eventos injetados** (`LLKHF_INJECTED`, inclusive os nossos) passam direto, sem tocar no estado → sem loop.
- **Hook mínimo**: o callback só decide bloquear/passar e enfileira no máximo 1 ação; desenhar, timers e `SendInput` rodam no loop de mensagens (o Windows remove hooks lentos silenciosamente).
- **Sem reentrância**: nenhum `borrow` de estado atravessa chamada Win32. Se o estado estiver ocupado (`try_borrow_mut` falha), a tecla passa.
- **Fail-safe**: `panic = "abort"` → processo morre → Windows remove o hook → teclado normal.
- **Instância única** via mutex nomeado; segunda instância sai em silêncio.
- **Bandeja resiliente**: reinsere o ícone ao receber `TaskbarCreated` (Explorer reiniciou).
- **Privacidade**: nenhuma tecla é logada ou gravada; zero rede. Logs de diagnóstico (`OutputDebugStringW`) nunca contêm teclas/caracteres.
- **Limitação conhecida** (igual PowerToys): não atua em janelas elevadas (admin), tela de bloqueio ou UAC — lá o teclado funciona normal.

## Arquitetura

Crate `acentorust` com lib + bin.

```
src/
  lib.rs        pub mod accents, engine, input, popup, tray, autostart
  accents.rs    tabela Letter → variantes (puro)
  engine.rs     máquina de estados (pura, sem Win32) — núcleo testado por TDD
  input.rs      SendInput: commit (Backspace + char Unicode) e replay (Espaço + tecla)
  popup.rs      janela da barra (GDI, DPI-aware, não rouba foco)
  tray.rs       ícone desenhado em runtime + menu
  autostart.rs  HKCU\Software\Microsoft\Windows\CurrentVersion\Run
  main.rs       bin: instância única, DPI, janela oculta, loop de mensagens, despacho de ações
  hook.rs       bin: WH_KEYBOARD_LL + Env real (Win32)
```

### Máquina de estados (`engine.rs`)

Estados: `Idle` · `Held { letter, upper }` · `Active { letter, upper, index, confirmed }`.

Entrada: `Event::{KeyDown(RawKey), KeyUp(RawKey), Timer}` + `&impl Env` (consulta de teclas pressionadas, Caps Lock, tela cheia — consultada preguiçosamente, só quando necessário).
Saída: `Outcome { block: bool, action: Option<Action> }`.
Ações: `Arm` (inicia timer de confirmação) · `Show { variants, index }` · `Hide` · `Commit(char)` · `Replay(Option<RawKey>)`. `Hide`/`Commit`/`Replay` encerram a sessão (o shell esconde a barra e cancela o timer).

A tabela de transições completa está no plano (Task 2).

### Fluxo de execução (single-thread)

```
teclado → hook LL (thread principal) → engine.handle() → block/pass
                                          └─ ação → fila (VecDeque) → PostMessage(WM_APP_FLUSH)
loop de mensagens → WM_APP_FLUSH → executa ações uma a uma (popup / SetTimer / SendInput)
                  → WM_TIMER     → engine.handle(Timer) → fila → flush
                  → bandeja      → menu → autostart / sair
```

### Barra (popup)

- Janela `WS_POPUP` com `WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE`; `WM_MOUSEACTIVATE → MA_NOACTIVATE`; mostrada com `SWP_NOACTIVATE`.
- Posição: centro horizontal da área de trabalho (`rcWork`) do monitor da janela em primeiro plano, 24 DIP abaixo do topo.
- Visual (DIP, escalado pelo DPI do monitor): painel `#18181B`; padding 6; células 44×52, gap 2; caractere Segoe UI semibold 24 px, cor `#E4E4E7`; célula selecionada: retângulo arredondado raio 6 em `#2F6FDB` com caractere `#FAFAFA`. Cantos arredondados do Windows 11 via `DWMWA_WINDOW_CORNER_PREFERENCE = DWMWCP_ROUND` e borda `#3F3F46` via `DWMWA_BORDER_COLOR` (falha ignorada no Windows 10).
- Pintura com double buffer (memory DC + `BitBlt`); `WM_ERASEBKGND` retorna 1. Sem animação (latência > estética; `AnimateWindow` bloquearia a thread do hook).

### Bandeja

Ícone desenhado em runtime (quadrado arredondado `#2F6FDB` com "á" branco), tooltip "AcentoRust". Clique esquerdo ou direito abre o menu: "Iniciar com o Windows" (check reflete o registro) · separador · "Sair".

### Autostart

Valor `AcentoRust` em `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` = `"<caminho do exe>"`. Ligado = valor existe. Na inicialização, se ligado, regrava com o caminho atual (exe movido continua funcionando).

## Dependências e build

- `windows-sys 0.61` (bindings oficiais da Microsoft, zero runtime) e `anyhow` (erros de inicialização).
- `[profile.release]`: `opt-level = "s"`, `lto = true`, `codegen-units = 1`, `panic = "abort"`, `strip = true`.
- Lints: `unsafe_op_in_unsafe_fn = deny`, `clippy::undocumented_unsafe_blocks = deny` (todo `unsafe` com `// SAFETY:`), `clippy::unwrap_used = deny` (permitido em testes).

## Erros

- Inicialização (hook, janelas, bandeja): `MessageBoxW` "AcentoRust: <erro com contexto>" e sai com código 1.
- Execução (SendInput parcial, registro): `OutputDebugStringW("AcentoRust: ...")` (visível no DebugView), sem dados de tecla; menu de autostart com falha mostra `MessageBoxW`.

## Testes

- TDD da `engine` cobrindo todos os cenários da tabela de transições (rollover, confirmação por timer e por navegação, wrap, Esc, outra tecla, autorepeat, maiúsculas, tela cheia, letra fantasma, modificadores).
- Testes da tabela `accents`.
- Camadas Win32: validadas por `cargo clippy -- -D warnings`, build release e roteiro manual (o programa ignora input injetado, então E2E automatizado por `SendInput` é impossível por design).
- Medir tamanho do `.exe` e working set em execução.
