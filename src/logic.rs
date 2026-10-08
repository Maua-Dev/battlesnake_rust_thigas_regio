// Bem-vindo ao
// __________         __    __  .__                               __
// \______   \_____ _/  |__/  |_|  |   ____   ______ ____ _____  |  | __ ____
//  |    |  _/\__  \   __\   __\  | _/ __ \ /  ___//    \__  \ |  |/ // __ \
//  |    |   \ / __ \|  |  |  | |  |_\  ___/ \___ \|   |  \/ __ \|    <\  ___/
//  |________/(______/__|  |__| |____/\_____>______>___|__(______/__|__\_____>
//
// As quatro funcoes que a arena chama. A inteligencia da cobra fica nos
// outros arquivos; aqui so organizamos a conversa com a arena.
// Documentacao: https://docs.battlesnake.com

// ============================================================================
// ||  BLOCO: CONTROLE DE TEMPO
// ||  O QUE FAZ: define quanto tempo a busca pode pensar em cada jogada.
// ||  POR QUE:   a arena espera no maximo 500 ms. Estourar o tempo faz a
// ||             cobra repetir a jogada anterior, o que costuma matar.
// ============================================================================

/// Tempo normal de busca por jogada.
#[cfg(not(test))]
const SEARCH_BUDGET_MS: u64 = 180;
// >>> Nos testes a busca pensa pouco, para a bateria de testes ser rapida.
#[cfg(test)]
const SEARCH_BUDGET_MS: u64 = 10;
/// Nunca usar mais que (timeout da partida - esta margem): sobra para a rede.
const RESPONSE_MARGIN_MS: u64 = 150;
/// Primeira jogada de um processo novo (cold start): pensar menos.
const COLD_START_BUDGET_MS: u64 = 60;
/// Se a arena mediu uma latencia acima disto na jogada anterior, pensar metade.
const SLOW_LATENCY_MS: u64 = 420;

use crate::board::Direction;
use crate::eval::{EvalWeights, EVAL, WIN};
use crate::models::GameState;
use crate::{board, safety, search};
use serde_json::{json, Value};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use tracing::info;

/// Fica `true` depois da primeira jogada atendida por este processo.
static WARMED_UP: AtomicBool = AtomicBool::new(false);

/// GET / — chamado quando você cadastra a cobra no site e a cada partida.
/// Controla a aparência dela. Opções de cabeça, cauda e cor:
/// https://docs.battlesnake.com/guides/customizations
pub fn info() -> Value {
    info!("INFO");

    json!({
        "apiversion": "1",
        "author": "Thiagogit46",
        "color": "#0ABAB5",    // azul Tiffany, escolha do Thiago
        "head": "tiger-king",  // TODO: escolha a cabeça
        "tail": "hook",        // TODO: escolha a cauda
        "version": "2026.10.06"
    })
}

/// POST /start — chamado uma vez, quando a partida começa.
pub fn start(state: &GameState) {
    info!("JOGO COMEÇOU (partida {})", state.game.id);
}

/// POST /end — chamado uma vez, quando a partida termina.
pub fn end(state: &GameState) {
    info!("FIM DE JOGO após {} turnos", state.turn);
}

// ============================================================================
// ||  BLOCO: JOGADA DO TURNO (POST /move)
// ||  O QUE FAZ: 1) a rede de seguranca escolhe uma jogada provisoria;
// ||             2) a busca pensa ate o prazo e, se completar pelo menos uma
// ||                profundidade, a jogada dela vale.
// ||  POR QUE:   sempre ha uma resposta pronta. Se a busca der panico ou
// ||             nao terminar, fica a jogada da rede de seguranca.
// ============================================================================

pub fn get_move(state: &GameState) -> Value {
    let deadline = Instant::now() + search_budget(state);
    let (direction, shout) = decide(state, deadline);
    info!("MOVE {}: {}", state.turn, shout);
    json!({ "move": direction.as_str(), "shout": shout })
}

/// A decisao completa (rede de seguranca + busca ate `deadline`).
pub fn decide(state: &GameState, deadline: Instant) -> (Direction, String) {
    decide_with(state, deadline, &EVAL)
}

/// A decisao com os pesos de avaliacao escolhidos (o self-play usa isto).
pub fn decide_with(state: &GameState, deadline: Instant, weights: &EvalWeights) -> (Direction, String) {
    let started = Instant::now();

    // >>> A arena manda cobras JA ELIMINADAS na lista (visto em 08/10). Tiramos
    // >>> antes de pensar; sem isso, um "cadaver" vira parede e adversaria.
    let (clean, removed) = match catch_unwind(AssertUnwindSafe(|| board::remove_eliminated(state))) {
        Ok(result) => result,
        Err(_) => (state.clone(), 0),
    };
    let notes = arena_notes(state, removed);
    let (direction, shout) = decide_clean(&clean, deadline, weights, started);
    (direction, format!("{shout}{notes}"))
}

/// Diagnostico para o shout: quantas mortas tiramos e os campos extras que a
/// arena mandou (para descobrir o formato dela pelos quadros das partidas).
fn arena_notes(state: &GameState, removed: usize) -> String {
    let mut keys: Vec<&str> = Vec::new();
    for snake in &state.board.snakes {
        for key in snake.extra.keys() {
            if !keys.contains(&key.as_str()) {
                keys.push(key.as_str());
            }
        }
    }
    keys.sort();
    let mut notes = String::new();
    if removed > 0 {
        notes.push_str(&format!(" mortas{removed}"));
    }
    if !keys.is_empty() {
        let mut list = keys.join(",");
        // >>> O shout aceita ate 256 caracteres; a lista nao pode estourar isso.
        list.truncate(80);
        notes.push_str(&format!(" x:{list}"));
    }
    notes
}

/// A decisao (rede de seguranca + busca) num estado ja sem cobras eliminadas.
fn decide_clean(state: &GameState, deadline: Instant, weights: &EvalWeights, started: Instant) -> (Direction, String) {

    // >>> `catch_unwind` segura um panico (erro grave) que aconteca la dentro.
    // >>> Sem ele, um panico derrubaria a resposta inteira.
    let safe = match catch_unwind(AssertUnwindSafe(|| safety::choose_move(state))) {
        Ok(decision) => decision,
        Err(_) => return (safety::emergency_move(state), "emergencia".to_string()),
    };

    let searched = catch_unwind(AssertUnwindSafe(|| search::search(state, deadline, &safe.ranking, weights)));
    match searched {
        Ok(Some(result)) => {
            // >>> Nota enorme = a busca viu o fim da partida: vitoria ou derrota garantida.
            let verdict = if result.score >= WIN / 2 {
                " ganha"
            } else if result.score <= -WIN / 2 {
                " perde"
            } else {
                ""
            };
            let shout = format!(
                "{} | {} {} prof{} nos{} {}ms{}",
                safe.shout,
                weights.tag,
                result.direction.as_str(),
                result.depth,
                result.nodes,
                started.elapsed().as_millis(),
                verdict
            );
            (result.direction, shout)
        }
        _ => (safe.direction, safe.shout),
    }
}

/// Quanto tempo a busca pode usar nesta jogada.
fn search_budget(state: &GameState) -> Duration {
    let timeout = state.game.timeout as u64;
    let mut budget = SEARCH_BUDGET_MS.min(timeout.saturating_sub(RESPONSE_MARGIN_MS));
    // >>> `swap` marca o processo como "aquecido" e devolve como estava antes.
    if !WARMED_UP.swap(true, Ordering::Relaxed) {
        budget = budget.min(COLD_START_BUDGET_MS);
    }
    let last_latency = state.you.latency.as_deref().and_then(|text| text.parse::<u64>().ok());
    if let Some(latency) = last_latency {
        if latency > SLOW_LATENCY_MS {
            budget /= 2;
        }
    }
    Duration::from_millis(budget)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Battlesnake, Board, Coord, Game};
    use rand::Rng;
    use std::collections::HashMap;

    /// Monta um estado de jogo mínimo para os testes, com a cobra deitada
    /// na horizontal: cabeça em `head` e pescoço em `neck`.
    fn game_state(head: Coord, neck: Coord) -> GameState {
        let you = Battlesnake {
            id: "minha-cobra".to_string(),
            name: "MinhaCobra".to_string(),
            health: 100,
            body: vec![head, neck, Coord { x: neck.x, y: neck.y - 1 }],
            head,
            length: 3,
            latency: Some("50".to_string()),
            shout: None,
            extra: Default::default(),
        };

        GameState {
            game: Game {
                id: "partida-de-teste".to_string(),
                ruleset: HashMap::new(),
                map: Some("standard".to_string()),
                timeout: 500,
            },
            turn: 4,
            board: Board {
                height: 11,
                width: 11,
                food: vec![Coord { x: 5, y: 5 }],
                hazards: vec![],
                snakes: vec![you.clone()],
            },
            you,
        }
    }

    fn chosen_move(state: &GameState) -> String {
        get_move(state)["move"].as_str().unwrap().to_string()
    }

    #[test]
    fn info_devolve_os_campos_obrigatorios() {
        let response = info();

        assert_eq!(response["apiversion"], "1");
        assert!(response.get("author").is_some());
        assert!(response.get("color").is_some());
        assert!(response.get("head").is_some());
        assert!(response.get("tail").is_some());
    }

    #[test]
    fn move_devolve_sempre_uma_direcao_valida() {
        let state = game_state(Coord { x: 5, y: 4 }, Coord { x: 4, y: 4 });

        for _ in 0..50 {
            let direction = chosen_move(&state);
            assert!(
                ["up", "down", "left", "right"].contains(&direction.as_str()),
                "direção inválida: {direction}"
            );
        }
    }

    #[test]
    fn nunca_volta_por_cima_do_pescoco() {
        // pescoço à esquerda da cabeça: "left" seria andar para trás
        let state = game_state(Coord { x: 5, y: 4 }, Coord { x: 4, y: 4 });
        for _ in 0..50 {
            assert_ne!(chosen_move(&state), "left");
        }

        // pescoço à direita da cabeça: "right" seria andar para trás
        let state = game_state(Coord { x: 5, y: 4 }, Coord { x: 6, y: 4 });
        for _ in 0..50 {
            assert_ne!(chosen_move(&state), "right");
        }

        // pescoço abaixo da cabeça: "down" seria andar para trás
        let state = game_state(Coord { x: 5, y: 4 }, Coord { x: 5, y: 3 });
        for _ in 0..50 {
            assert_ne!(chosen_move(&state), "down");
        }

        // pescoço acima da cabeça: "up" seria andar para trás
        let state = game_state(Coord { x: 5, y: 4 }, Coord { x: 5, y: 5 });
        for _ in 0..50 {
            assert_ne!(chosen_move(&state), "up");
        }
    }

    #[test]
    fn evita_parede_quando_tem_opcao() {
        // Cobra no canto inferior esquerdo, pescoço à direita da cabeça:
        // não pode ir para right (pescoço) nem left (x=-1) nem down (y=-1).
        // A única opção segura é "up".
        let state = game_state(Coord { x: 0, y: 0 }, Coord { x: 1, y: 0 });
        for _ in 0..50 {
            let direction = chosen_move(&state);
            assert!(
                ["up", "down", "left", "right"].contains(&direction.as_str()),
                "direção inválida: {direction}"
            );
            assert_ne!(direction, "left",  "foi para fora do tabuleiro (esquerda)");
            assert_ne!(direction, "down",  "foi para fora do tabuleiro (baixo)");
        }
    }

    #[test]
    fn evita_proprio_corpo_quando_tem_opcao() {
        // Cabeça em (5,4), pescoço à esquerda (4,4), corpo acima em (5,5).
        // Restam right e down. Verificamos que nunca escolhe "left" nem "up".
        let head = Coord { x: 5, y: 4 };
        let neck = Coord { x: 4, y: 4 };
        let mut state = game_state(head, neck);
        state.you.body = vec![head, neck, Coord { x: 5, y: 5 }, Coord { x: 4, y: 3 }];
        state.board.snakes = vec![state.you.clone()];

        for _ in 0..50 {
            let direction = chosen_move(&state);
            assert_ne!(direction, "left", "voltou pelo pescoço");
            assert_ne!(direction, "up", "bateu no próprio corpo");
            assert!(["right", "down"].contains(&direction.as_str()));
        }
    }

    #[test]
    fn comportamento_definido_sem_safe_moves() {
        // Cabeça no canto (0,0), pescoço acima (0,1) — bloqueia up.
        // left (x=-1) e down (y=-1) saem do tabuleiro.
        // Somente right estaria livre, mas o helper adiciona um segmento
        // em (1,0) para fechar todas as saídas e testar o fallback.
        //
        // Independentemente de qual direção for escolhida, não pode lançar
        // pânico e deve ser uma das quatro direções válidas.
        let head = Coord { x: 0, y: 0 };
        let neck = Coord { x: 0, y: 1 };
        let mut state = game_state(head, neck);
        // Adiciona um segmento do corpo à direita para bloquear "right"
        state.you.body.push(Coord { x: 1, y: 0 });
        state.board.snakes = vec![state.you.clone()];

        let direction = chosen_move(&state);
        assert!(
            ["up", "down", "left", "right"].contains(&direction.as_str()),
            "fallback retornou direção inválida: {direction}"
        );
    }

    // ------------------------------------------------------------------------
    // Robustez: centenas de tabuleiros aleatórios, inclusive absurdos
    // (corpos fora do tabuleiro, vida negativa, tabuleiro 1x1). A resposta
    // tem que ser sempre uma das 4 direções, sem pânico.
    // ------------------------------------------------------------------------

    fn random_snake(rng: &mut rand::rngs::ThreadRng, id: &str, width: i32, height: i32) -> Battlesnake {
        let length = rng.random_range(0..=20);
        let mut body = Vec::new();
        let mut cell = Coord {
            x: rng.random_range(-1..=width),
            y: rng.random_range(-1..=height),
        };
        for _ in 0..length {
            body.push(cell);
            match rng.random_range(0..5) {
                0 => cell.x += 1,
                1 => cell.x -= 1,
                2 => cell.y += 1,
                3 => cell.y -= 1,
                _ => {} // repete a casa, como uma cauda empilhada
            }
        }
        let head = match body.first() {
            Some(first) => *first,
            None => Coord { x: 0, y: 0 },
        };
        Battlesnake {
            id: id.to_string(),
            name: id.to_string(),
            health: rng.random_range(-5..=105),
            length: body.len() as i32,
            body,
            head,
            latency: None,
            shout: None,
            extra: Default::default(),
        }
    }

    #[test]
    fn robustez_tabuleiros_aleatorios_sempre_respondem_direcao_valida() {
        let mut rng = rand::rng();
        for round in 0..500 {
            let width = rng.random_range(0..=19);
            let height = rng.random_range(0..=19);
            let snake_count = rng.random_range(1..=8);
            let mut snakes = Vec::new();
            for i in 0..snake_count {
                snakes.push(random_snake(&mut rng, &format!("cobra-{i}"), width, height));
            }
            let you = snakes[0].clone();
            // >>> Às vezes a nossa cobra NÃO vem na lista, para testar esse caso também.
            if round % 7 == 0 {
                snakes.remove(0);
            }
            let mut food = Vec::new();
            for _ in 0..rng.random_range(0..=10) {
                food.push(Coord {
                    x: rng.random_range(-1..=width),
                    y: rng.random_range(-1..=height),
                });
            }

            let state = GameState {
                game: Game {
                    id: "aleatoria".to_string(),
                    ruleset: HashMap::new(),
                    map: None,
                    timeout: 500,
                },
                turn: rng.random_range(0..=300),
                board: Board { height, width, food, hazards: vec![], snakes },
                you,
            };

            // >>> Chamamos a decisão DIRETO (sem o catch_unwind do get_move):
            // >>> assim um pânico aparece como falha do teste em vez de ser escondido.
            let decision = crate::safety::choose_move(&state);
            assert!(["up", "down", "left", "right"].contains(&decision.direction.as_str()));

            let direction = chosen_move(&state);
            assert!(
                ["up", "down", "left", "right"].contains(&direction.as_str()),
                "rodada {round}: direção inválida {direction}"
            );
        }
    }
}

#[cfg(test)]
mod arena_mortas {
    use crate::models::GameState;
    use serde_json::json;
    use std::time::{Duration, Instant};

    /// Partida 4e1ffb00 (3 cobras), turno 87. A "THE BODÃO AO QUADRADO" morreu
    /// na parede no turno 10, mas a arena continuou mandando o corpo dela na
    /// lista. A b0707 achou que (5,9) estava ocupada, respondeu "sem saida" e
    /// bateu no proprio corpo. A saida certa e a esquerda.
    ///
    /// y=10  e e e e e m b b . . .     m = cadaver (cabeca fora, em (5,11))
    /// y= 9  E e e e e m H b . . .     H = nos
    /// y= 8  . . . . . m b b . . .
    #[test]
    fn nao_trata_cobra_morta_como_parede() {
        let c = |v: &[(i32, i32)]| v.iter().map(|(x, y)| json!({"x": x, "y": y})).collect::<Vec<_>>();
        let snake = |id: &str, health: i32, body: &[(i32, i32)], cause: &str| json!({"id": id, "name": id, "health": health,
            "body": c(body), "head": c(body)[0], "length": body.len(), "latency": "200", "shout": "", "EliminatedCause": cause});
        let me = snake("eu", 98, &[(6, 9), (6, 10), (7, 10), (7, 9), (7, 8), (6, 8), (6, 7), (6, 6)], "");
        let corpse = snake("morta", 94, &[(5, 11), (5, 10), (5, 9), (5, 8)], "wall-collision");
        let bodao = snake("bodao", 90, &[(0, 9), (0, 10), (1, 10), (2, 10), (3, 10), (4, 10), (4, 9), (3, 9), (2, 9), (1, 9)], "");
        let s: GameState = serde_json::from_value(json!({"game": {"id": "g", "ruleset": {}, "timeout": 500}, "turn": 87,
            "board": {"width": 11, "height": 11, "food": c(&[(5, 10)]), "hazards": [], "snakes": [corpse, me.clone(), bodao]},
            "you": me})).unwrap();
        let (direction, shout) = super::decide(&s, Instant::now() + Duration::from_millis(100));
        assert_eq!(direction.as_str(), "left", "{shout}");
        assert!(shout.contains("mortas1"), "{shout}");
    }
}

/// Ferramenta de depuracao: rejoga turnos de uma partida real da arena.
/// O arquivo (uma requisicao /move por linha) sai de um script que converte os
/// quadros baixados. Rode com:
/// REPLAY=caminho.jsonl cargo test --release rejogar_partida -- --ignored --nocapture
#[cfg(test)]
mod replay {
    use crate::models::GameState;
    use std::time::{Duration, Instant};

    #[test]
    #[ignore]
    fn rejogar_partida() {
        let path = match std::env::var("REPLAY") {
            Ok(path) => path,
            Err(_) => return,
        };
        let budget: u64 = std::env::var("BUDGET_MS").ok().and_then(|v| v.parse().ok()).unwrap_or(180);
        let text = std::fs::read_to_string(path).unwrap_or_default();
        for line in text.lines() {
            let state: GameState = match serde_json::from_str(line) {
                Ok(state) => state,
                Err(_) => continue,
            };
            let (direction, shout) = super::decide(&state, Instant::now() + Duration::from_millis(budget));
            println!("turno {:3} -> {:5} {}", state.turn, direction.as_str(), shout);
        }
    }
}
