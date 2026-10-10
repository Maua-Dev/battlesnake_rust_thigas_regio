// ============================================================================
// ||  BLOCO: SELF-PLAY (SO EM TESTE)
// ||  O QUE FAZ: joga partidas inteiras aqui no computador, com comida
// ||             aparecendo ao acaso, entre a nossa cobra e bots de referencia.
// ||  POR QUE:   para medir se uma mudanca melhora a cobra ANTES do deploy.
// ||  COMO RODAR: cargo test --release -- --ignored --nocapture
// ============================================================================

use crate::board::Direction;
use crate::models::{Battlesnake, Board, Coord, Game, GameState};
use crate::rules;
use crate::eval::{EvalWeights, EVAL};
use crate::safety::{self, Settings, SETTINGS, SETTINGS_V0510};
use rand::rngs::StdRng;
use rand::seq::{IndexedRandom, SliceRandom};
use rand::{Rng, SeedableRng};
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// As partidas locais param aqui, se ninguem morrer antes. Na arena nao ha
/// limite pratico (a partida mais longa que vimos foi ate o turno 603).
const SELFPLAY_MAX_TURNS: i32 = 500;

/// Regras de comida do modo standard: sempre pelo menos 1 comida, e 15% de
/// chance de nascer mais uma a cada turno.
const MINIMUM_FOOD: usize = 1;
const FOOD_SPAWN_CHANCE: u32 = 15;

/// Quem controla cada cobra numa partida de teste.
#[derive(Clone, Copy)]
enum Player {
    /// A nossa cobra, com os pesos de uma versao.
    Ours(Settings),
    /// Nunca se mata de imediato, mas escolhe ao acaso.
    RandomSafe,
    /// Vai sempre para a comida mais perto.
    Greedy,
    /// Gira atras da propria cauda e so sai para comer com vida baixa.
    Chicken,
    /// Chicken cuidadosa (parecida com a tenhoTDAH da arena): tambem foge de
    /// cabeca com cabeca e de becos, e come mais cedo.
    CarefulChicken,
    /// A cobra completa (rede de seguranca + busca), pensando tantos ms por jogada.
    Search(u64, EvalWeights),
}

impl Player {
    fn name(&self) -> String {
        match self {
            Player::Ours(settings) => settings.tag.to_string(),
            Player::RandomSafe => "aleatoria".to_string(),
            Player::Greedy => "gulosa".to_string(),
            Player::Chicken => "chicken".to_string(),
            Player::CarefulChicken => "cuidadosa".to_string(),
            Player::Search(ms, weights) => format!("{}{ms}", weights.tag),
        }
    }
}

/// Como terminou uma partida.
struct Outcome {
    /// Indices (na lista de jogadores) de quem estava vivo no fim.
    survivors: Vec<usize>,
    /// Tamanho de cada jogador no fim (0 se morreu).
    lengths: Vec<i32>,
    turns: i32,
    /// Maior tempo de decisao da NOSSA cobra, em milissegundos.
    slowest_ms: f64,
}

// ----------------------------------------------------------------------------
// Montagem da partida
// ----------------------------------------------------------------------------

/// Partida nova no 11x11: cobras empilhadas a uma casa da borda, uma comida
/// perto de cada uma e uma no centro (como vimos nos quadros da arena).
fn new_game(player_count: usize, rng: &mut StdRng) -> GameState {
    let mut spawns = vec![(1, 1), (1, 5), (1, 9), (5, 1), (5, 9), (9, 1), (9, 5), (9, 9)];
    spawns.shuffle(rng);

    let mut snakes = Vec::new();
    let mut food = vec![Coord { x: 5, y: 5 }];
    for (i, (x, y)) in spawns.iter().take(player_count).enumerate() {
        let start = Coord { x: *x, y: *y };
        snakes.push(Battlesnake {
            id: i.to_string(),
            name: i.to_string(),
            health: 100,
            body: vec![start, start, start],
            head: start,
            length: 3,
            latency: None,
            shout: None,
            extra: Default::default(),
        });
        // Uma comida numa das diagonais da cobra.
        let mut options: Vec<Coord> = Vec::new();
        for (dx, dy) in [(-1, -1), (-1, 1), (1, -1), (1, 1)] {
            let c = Coord { x: x + dx, y: y + dy };
            if c.x >= 0 && c.y >= 0 && c.x < 11 && c.y < 11 && !food.contains(&c) && c != (Coord { x: 5, y: 5 }) {
                options.push(c);
            }
        }
        if let Some(choice) = options.choose(rng) {
            food.push(*choice);
        }
    }

    GameState {
        game: Game { id: "selfplay".to_string(), ruleset: HashMap::new(), map: None, timeout: 500 },
        turn: 0,
        board: Board { width: 11, height: 11, food, hazards: vec![], snakes: snakes.clone() },
        you: snakes[0].clone(),
    }
}

/// Faz nascer comida numa casa vazia, seguindo as regras do modo standard.
fn spawn_food(state: &mut GameState, rng: &mut StdRng) {
    let wanted = if state.board.food.len() < MINIMUM_FOOD {
        MINIMUM_FOOD - state.board.food.len()
    } else if rng.random_range(0..100) < FOOD_SPAWN_CHANCE {
        1
    } else {
        0
    };
    for _ in 0..wanted {
        let mut free = Vec::new();
        for x in 0..state.board.width {
            for y in 0..state.board.height {
                let c = Coord { x, y };
                let taken = state.board.food.contains(&c)
                    || state.board.snakes.iter().any(|s| s.body.contains(&c));
                if !taken {
                    free.push(c);
                }
            }
        }
        if let Some(choice) = free.choose(rng) {
            state.board.food.push(*choice);
        }
    }
}

// ----------------------------------------------------------------------------
// Decisao de cada tipo de jogador
// ----------------------------------------------------------------------------

fn manhattan(a: Coord, b: Coord) -> i32 {
    (a.x - b.x).abs() + (a.y - b.y).abs()
}

fn nearest_food_distance(state: &GameState, c: Coord) -> i32 {
    state.board.food.iter().map(|f| manhattan(*f, c)).min().unwrap_or(0)
}

fn decide(player: &Player, view: &GameState, rng: &mut StdRng) -> Direction {
    if let Player::Ours(settings) = player {
        return safety::choose_move_with(view, settings).direction;
    }
    if let Player::Search(ms, weights) = player {
        return crate::logic::decide_with(view, Instant::now() + Duration::from_millis(*ms), weights).0;
    }

    // Os bots usam a nossa analise so para saber o que e legal e o que e beco.
    let infos = safety::analyze_moves_with(view, &SETTINGS_V0510);
    let legal: Vec<&safety::MoveInfo> = infos.iter().filter(|i| i.legal).collect();
    if legal.is_empty() {
        return Direction::Up;
    }

    match player {
        Player::RandomSafe => legal.choose(rng).map(|i| i.direction).unwrap_or(Direction::Up),
        Player::Greedy => {
            let best = legal.iter().map(|i| nearest_food_distance(view, i.target)).min().unwrap_or(0);
            let options: Vec<&&safety::MoveInfo> =
                legal.iter().filter(|i| nearest_food_distance(view, i.target) == best).collect();
            options.choose(rng).map(|i| i.direction).unwrap_or(Direction::Up)
        }
        Player::Chicken | Player::CarefulChicken => {
            let careful = matches!(player, Player::CarefulChicken);
            let open: Vec<&&safety::MoveInfo> = legal.iter().filter(|i| !i.trapped && !(careful && i.head_risk)).collect();
            let open = if open.is_empty() { legal.iter().filter(|i| !i.trapped).collect() } else { open };
            let pool: Vec<&&safety::MoveInfo> = if open.is_empty() { legal.iter().collect() } else { open };
            let hungry_below = if careful { 40 } else { 25 };
            if view.you.health < hungry_below {
                // Com fome: sai do canto para comer.
                let mut best = pool[0];
                for i in &pool {
                    if nearest_food_distance(view, i.target) < nearest_food_distance(view, best.target) {
                        best = i;
                    }
                }
                return best.direction;
            }
            // Sem fome: segue a propria cauda e evita comer.
            let tail = view.you.body.last().copied().unwrap_or(view.you.head);
            let mut best = pool[0];
            let mut best_key = (true, i32::MAX);
            for i in &pool {
                let key = (i.eats, manhattan(i.target, tail));
                if key < best_key {
                    best_key = key;
                    best = i;
                }
            }
            best.direction
        }
        Player::Ours(_) | Player::Search(_, _) => Direction::Up,
    }
}

// ----------------------------------------------------------------------------
// Uma partida inteira
// ----------------------------------------------------------------------------

fn play_game(players: &[Player], rng: &mut StdRng) -> Outcome {
    let mut state = new_game(players.len(), rng);
    let mut slowest_ms: f64 = 0.0;

    while state.board.snakes.len() > 1 && state.turn < SELFPLAY_MAX_TURNS {
        let mut moves = Vec::new();
        for snake in &state.board.snakes {
            let index: usize = snake.id.parse().unwrap_or(0);
            let mut view = state.clone();
            view.you = snake.clone();
            let started = Instant::now();
            let direction = decide(&players[index], &view, rng);
            if let Player::Ours(_) | Player::Search(_, _) = players[index] {
                slowest_ms = slowest_ms.max(started.elapsed().as_secs_f64() * 1000.0);
            }
            moves.push(direction);
        }
        rules::apply_turn(&mut state, &moves);
        spawn_food(&mut state, rng);
    }

    let mut lengths = vec![0; players.len()];
    let mut survivors = Vec::new();
    for snake in &state.board.snakes {
        let index: usize = snake.id.parse().unwrap_or(0);
        survivors.push(index);
        lengths[index] = snake.body.len() as i32;
    }
    Outcome { survivors, lengths, turns: state.turn, slowest_ms }
}

/// Quantas partidas rodam ao mesmo tempo (uma por "thread" do processador).
const PARALLEL_GAMES: usize = 6;

/// Joga `games` partidas, varias ao mesmo tempo. Cada partida tem a sua
/// propria semente, entao o resultado nao depende da ordem das threads.
fn play_many(players: &[Player], games: usize, seed: u64) -> Vec<Outcome> {
    let mut outcomes = Vec::new();
    std::thread::scope(|scope| {
        let mut handles = Vec::new();
        for thread in 0..PARALLEL_GAMES {
            let players = players.to_vec();
            handles.push(scope.spawn(move || {
                let mut mine = Vec::new();
                let mut game = thread;
                while game < games {
                    let mut rng = StdRng::seed_from_u64(seed * 100_000 + game as u64);
                    mine.push(play_game(&players, &mut rng));
                    game += PARALLEL_GAMES;
                }
                mine
            }));
        }
        for handle in handles {
            outcomes.extend(handle.join().unwrap_or_default());
        }
    });
    outcomes
}

/// Joga varias partidas e imprime, para cada jogador: vitorias por eliminacao,
/// sobrevivencias ate o fim (SELFPLAY_MAX_TURNS) e mortes.
/// Devolve a pontuacao do jogador 0: vitoria = 1, viva no fim = 1/(vivos), morte = 0.
fn run_match(label: &str, players: &[Player], games: usize, seed: u64) -> f64 {
    let count = players.len();
    let mut sole_wins = vec![0; count];
    let mut cap_alive = vec![0; count];
    let mut cap_biggest = vec![0; count];
    let mut deaths = vec![0; count];
    let mut points = vec![0.0; count];
    let mut total_turns = 0;
    let mut slowest: f64 = 0.0;

    for outcome in play_many(players, games, seed) {
        total_turns += outcome.turns;
        slowest = slowest.max(outcome.slowest_ms);
        let alive = outcome.survivors.len();
        for i in 0..count {
            if !outcome.survivors.contains(&i) {
                deaths[i] += 1;
            } else if alive == 1 {
                sole_wins[i] += 1;
                points[i] += 1.0;
            } else {
                cap_alive[i] += 1;
                points[i] += 1.0 / alive as f64;
                let biggest = outcome.survivors.iter().all(|j| *j == i || outcome.lengths[*j] < outcome.lengths[i]);
                if biggest {
                    cap_biggest[i] += 1;
                }
            }
        }
    }

    println!("\n== {label}: {games} partidas, media de {} turnos, decisao mais lenta {:.1} ms",
        total_turns / games as i32, slowest);
    println!("   {:10} {:>9} {:>14} {:>16} {:>7} {:>7}", "jogador", "eliminou", "viva no fim", "(maior no fim)", "morreu", "pontos");
    for i in 0..count {
        println!(
            "   {:10} {:>9} {:>14} {:>16} {:>7} {:>6.0}%",
            players[i].name(),
            sole_wins[i],
            cap_alive[i],
            cap_biggest[i],
            deaths[i],
            100.0 * points[i] / games as f64
        );
    }
    points[0] / games as f64
}

#[test]
fn selfplay_curto_roda_sem_panico() {
    // Poucas partidas, para rodar sempre no CI: so garante que nada quebra.
    let players = [Player::Ours(SETTINGS), Player::Ours(SETTINGS_V0510), Player::Greedy, Player::Chicken];
    run_match("teste rapido (4 cobras)", &players, 3, 7);
    run_match("teste rapido (duelo)", &[Player::Ours(SETTINGS), Player::RandomSafe], 3, 8);
}

/// Calibracao: varias combinacoes de pesos contra as mesmas adversarias.
/// Imprime a pontuacao (vitoria = 1, viva no fim = 1/vivas) de cada uma.
#[test]
#[ignore]
fn selfplay_calibracao() {
    let base = Settings { food_weight: 300, wanted_length_lead: 3, ..SETTINGS };
    let candidates = [
        Settings { tag: "l3", ..base },
        Settings { tag: "l3t5", territory_weight: 5, ..base },
        Settings { tag: "l3h40", hunt_weight: 40, ..base },
        Settings { tag: "l3h0", hunt_weight: 0, ..base },
        Settings { tag: "l4", wanted_length_lead: 4, ..base },
        Settings { tag: "l3f60", hungry_health: 60, ..base },
        Settings { tag: "l3k1000", kill_bonus: 1_000, ..base },
        Settings { tag: "l3cerco0", trap_enemy_bonus: 0, ..base },
    ];
    let old = Player::Ours(SETTINGS_V0510);
    println!();
    for candidate in candidates {
        let me = Player::Ours(candidate);
        let a = run_match_quiet(&[me, old], 300, 11);
        let b = run_match_quiet(&[me, Player::Chicken], 150, 12);
        let c = run_match_quiet(&[me, Player::Greedy], 150, 13);
        let d = run_match_quiet(&[me, old, Player::Greedy, Player::Chicken], 150, 14);
        println!(
            "{:10} x v0510 {:3.0}%   x chicken {:3.0}%   x gulosa {:3.0}%   4 cobras {:3.0}%",
            candidate.tag, a * 100.0, b * 100.0, c * 100.0, d * 100.0
        );
    }
}

/// Igual a `run_match`, mas sem imprimir: so devolve a pontuacao do jogador 0.
fn run_match_quiet(players: &[Player], games: usize, seed: u64) -> f64 {
    let mut points = 0.0;
    for outcome in play_many(players, games, seed) {
        if outcome.survivors.contains(&0) {
            points += 1.0 / outcome.survivors.len() as f64;
        }
    }
    points / games as f64
}

/// Mede quanto tempo a decisao completa leva, numa thread so, em tabuleiros
/// de partidas reais do self-play. Mostra o pior caso e os mais lentos.
#[test]
#[ignore]
fn tempo_da_busca() {
    let budget = 50;
    let players = [Player::Ours(SETTINGS), Player::Ours(SETTINGS), Player::Greedy, Player::Chicken];
    let mut times: Vec<(f64, i32, usize)> = Vec::new();
    for game in 0..6 {
        let mut rng = StdRng::seed_from_u64(900 + game);
        let count = if game % 2 == 0 { 2 } else { 4 };
        let mut state = new_game(count, &mut rng);
        while state.board.snakes.len() > 1 && state.turn < SELFPLAY_MAX_TURNS {
            let mut moves = Vec::new();
            for (i, snake) in state.board.snakes.iter().enumerate() {
                let mut view = state.clone();
                view.you = snake.clone();
                if i == 0 {
                    let started = Instant::now();
                    crate::logic::decide(&view, started + Duration::from_millis(budget));
                    times.push((started.elapsed().as_secs_f64() * 1000.0, state.turn, count));
                }
                let index: usize = snake.id.parse().unwrap_or(0);
                moves.push(decide(&players[index], &view, &mut rng));
            }
            rules::apply_turn(&mut state, &moves);
            spawn_food(&mut state, &mut rng);
        }
    }
    times.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    println!("\n{} decisoes, orcamento {budget} ms. As 8 mais lentas (ms, turno, cobras):", times.len());
    for (ms, turn, count) in times.iter().take(8) {
        println!("   {ms:7.1} ms  turno {turn:3}  {count} cobras");
    }
}

/// Calibracao da busca: varias combinacoes de pesos contra a v0606, que
/// sobrevive bem (parecida com as Tokuji). O que importa: eliminar sem morrer.
#[test]
#[ignore]
fn selfplay_calibracao_busca() {
    let candidates = [
        // 07/10: caca 100, 300 e 1000 eliminaram MENOS que a caca 20 (a cobra
        // persegue a cabeca e esquece de comer). Fica a base.
        // 08/10: territorio 20 e 40 ficaram iguais ou piores que a base
        // (x v0606: 66% base, 64% ter20, 62% ter40). Fica a base.
        EvalWeights { tag: "base", ..EVAL },
    ];
    for weights in candidates {
        run_match(weights.tag, &[Player::Search(50, weights), Player::Ours(SETTINGS)], 150, 51);
        run_match(weights.tag, &[Player::Search(50, weights), Player::CarefulChicken], 60, 52);
        run_match(weights.tag, &[Player::Search(50, weights), Player::Ours(SETTINGS), Player::Greedy, Player::CarefulChicken], 60, 53);
    }
}

/// Versao curta do teste abaixo, para comparar ajustes de pesos da avaliacao.
#[test]
#[ignore]
fn selfplay_busca_rapido() {
    let search = Player::Search(50, EVAL);
    run_match("busca x v0606 (duelo)", &[search, Player::Ours(SETTINGS)], 72, 31);
    run_match("busca x gulosa", &[search, Player::Greedy], 36, 32);
}

/// A cobra com busca contra as versoes sem busca e os bots.
#[test]
#[ignore]
fn selfplay_busca() {
    let search = Player::Search(50, EVAL);
    run_match("busca x v0606 (duelo)", &[search, Player::Ours(SETTINGS)], 120, 21);
    run_match("busca x v0510 (duelo)", &[search, Player::Ours(SETTINGS_V0510)], 120, 22);
    run_match("busca x chicken", &[search, Player::Chicken], 60, 23);
    run_match("busca x gulosa", &[search, Player::Greedy], 60, 24);
    run_match("4 cobras: busca, v0606, gulosa, chicken", &[search, Player::Ours(SETTINGS), Player::Greedy, Player::Chicken], 60, 25);
}

#[test]
#[ignore]
fn selfplay_relatorio() {
    let new = Player::Ours(SETTINGS);
    let old = Player::Ours(SETTINGS_V0510);
    run_match("v0606 x v0510 (duelo)", &[new, old], 300, 1);
    run_match("v0606 x chicken", &[new, Player::Chicken], 200, 2);
    run_match("v0510 x chicken", &[old, Player::Chicken], 200, 2);
    run_match("v0606 x gulosa", &[new, Player::Greedy], 200, 3);
    run_match("v0510 x gulosa", &[old, Player::Greedy], 200, 3);
    run_match("v0606 x aleatoria", &[new, Player::RandomSafe], 200, 4);
    run_match("4 cobras: v0606, v0510, gulosa, chicken", &[new, old, Player::Greedy, Player::Chicken], 200, 5);
}


/// Contra a chicken cuidadosa (estilo tenhoTDAH): pensar mais = eliminar mais?
#[test]
#[ignore]
fn selfplay_cuidadosa() {
    for ms in [25, 50, 100] {
        run_match(&format!("busca {ms} ms x cuidadosa"), &[Player::Search(ms, EVAL), Player::CarefulChicken], 60, 61);
    }
}
