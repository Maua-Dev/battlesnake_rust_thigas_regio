// ============================================================================
// ||  BLOCO: LIMITES DA BUSCA
// ||  O QUE FAZ: quanto a busca pode crescer e de quanto em quanto tempo
// ||             ela olha o relogio.
// ============================================================================

/// Profundidade maxima (em turnos). Na pratica o relogio para antes.
const MAX_DEPTH: i32 = 30;
/// Olhar o relogio a cada tantos tabuleiros (olhar sempre gasta tempo).
const NODES_PER_CLOCK_CHECK: u64 = 32;
/// Com 3 ou mais cobras, so as adversarias ate esta distancia da nossa
/// cabeca "jogam contra nos" na busca (no maximo MAX_MINIMIZERS delas).
const MINIMIZER_DISTANCE: i32 = 6;
const MAX_MINIMIZERS: usize = 2;
/// Um numero maior que qualquer nota possivel.
const INFINITY: i64 = i64::MAX / 4;

use crate::board::{manhattan, Direction, ALL_DIRECTIONS};
use crate::eval::{evaluate, terminal_value, EvalWeights, WIN};
use crate::models::{Coord, GameState};
use crate::rules::{apply_turn, ARENA_LAST_TURN};
use std::time::Instant;

/// O que a busca decidiu e quanto ela conseguiu enxergar.
pub struct SearchResult {
    pub direction: Direction,
    /// Profundidade completa mais funda (em turnos).
    pub depth: i32,
    /// Nota da jogada escolhida.
    pub score: i64,
    /// Quantos tabuleiros foram olhados.
    pub nodes: u64,
}

/// Guarda o estado da busca enquanto ela roda.
struct Searcher {
    me_id: String,
    weights: EvalWeights,
    deadline: Instant,
    nodes: u64,
    out_of_time: bool,
}

// ============================================================================
// ||  BLOCO: APROFUNDAMENTO ITERATIVO
// ||  O QUE FAZ: busca com profundidade 1, depois 2, depois 3... ate o prazo.
// ||  POR QUE:   assim sempre temos uma resposta pronta. Uma profundidade
// ||             interrompida pelo relogio e descartada; vale a ultima completa.
// ============================================================================

/// `root_order`: as nossas jogadas legais, da melhor para a pior segundo a
/// rede de seguranca. A busca comeca por elas, nessa ordem.
pub fn search(state: &GameState, deadline: Instant, root_order: &[Direction], weights: &EvalWeights) -> Option<SearchResult> {
    // >>> A busca copia o tabuleiro milhares de vezes. Copiando so o essencial
    // >>> (sem regras, nomes e textos), cada copia fica muito mais barata.
    let lean = lean_copy(state);
    let state = &lean;
    let me_id = state.you.id.as_str();
    let me_alive = state.board.snakes.iter().any(|s| s.id == me_id);
    // >>> Sem adversarias (partida solo) a busca nao tem o que decidir.
    if !me_alive || state.board.snakes.len() < 2 || root_order.is_empty() {
        return None;
    }

    let mut searcher = Searcher { me_id: me_id.to_string(), weights: *weights, deadline, nodes: 0, out_of_time: false };
    let mut order: Vec<Direction> = root_order.to_vec();
    let mut best: Option<SearchResult> = None;
    // >>> Depois do turno 99 nao ha jogo: nao adianta olhar mais longe que isso.
    let max_depth = MAX_DEPTH.min(ARENA_LAST_TURN - state.turn).max(1);

    for depth in 1..=max_depth {
        let mut alpha = -INFINITY;
        let mut depth_best: Option<(Direction, i64)> = None;
        for direction in order.iter().copied() {
            let value = searcher.min_value(state, direction, depth, alpha, INFINITY);
            if searcher.out_of_time {
                break;
            }
            if depth_best.map_or(true, |(_, best_value)| value > best_value) {
                depth_best = Some((direction, value));
            }
            alpha = alpha.max(value);
        }
        if searcher.out_of_time {
            break;
        }
        if let Some((direction, value)) = depth_best {
            best = Some(SearchResult { direction, depth, score: value, nodes: searcher.nodes });
            // A melhor jogada desta profundidade e a primeira a ser olhada na proxima.
            order.retain(|d| *d != direction);
            order.insert(0, direction);
            // >>> Vitoria ou derrota garantidas: olhar mais fundo nao muda nada.
            if value.abs() >= WIN / 2 {
                break;
            }
        }
    }

    if let Some(result) = best.as_mut() {
        result.nodes = searcher.nodes;
    }
    best
}

// ============================================================================
// ||  BLOCO: MINIMAX COM PODA ALFA-BETA
// ||  O QUE FAZ: nos escolhemos a jogada de MAIOR nota; as adversarias,
// ||             respondendo ja sabendo a nossa jogada, a de MENOR nota.
// ||  POR QUE:   e o jeito pessimista de tratar jogadas simultaneas: so
// ||             escolhemos o que funciona mesmo no pior caso.
// ||  PODA:      alfa = o minimo que ja garantimos; beta = o maximo que a
// ||             adversaria ja garantiu. Quando alfa >= beta, o resto daquele
// ||             galho nao pode mudar a decisao e e pulado.
// ============================================================================

impl Searcher {
    /// Vez da nossa cobra: a melhor nota entre as nossas jogadas.
    fn max_value(&mut self, state: &GameState, depth: i32, mut alpha: i64, beta: i64) -> i64 {
        self.nodes += 1;
        if self.nodes % NODES_PER_CLOCK_CHECK == 0 && Instant::now() >= self.deadline {
            self.out_of_time = true;
            return 0;
        }
        if let Some(value) = terminal_value(state, &self.me_id) {
            return value;
        }
        if depth <= 0 {
            return evaluate(state, &self.me_id, &self.weights);
        }
        let me = match state.board.snakes.iter().position(|s| s.id == self.me_id) {
            Some(index) => index,
            None => return -WIN,
        };

        let mut best = -INFINITY;
        for direction in legal_moves(state, me) {
            let value = self.min_value(state, direction, depth, alpha, beta);
            if self.out_of_time {
                return 0;
            }
            best = best.max(value);
            alpha = alpha.max(best);
            if alpha >= beta {
                break;
            }
        }
        best
    }

    /// Vez das adversarias, ja sabendo que vamos para `my_move`:
    /// a pior nota (para nos) entre as respostas delas. Depois aplica o turno.
    fn min_value(&mut self, state: &GameState, my_move: Direction, depth: i32, alpha: i64, mut beta: i64) -> i64 {
        let me = match state.board.snakes.iter().position(|s| s.id == self.me_id) {
            Some(index) => index,
            None => return -WIN,
        };

        // Jogada padrao de cada cobra; a nossa e `my_move`.
        let mut base_moves: Vec<Direction> = Vec::new();
        for index in 0..state.board.snakes.len() {
            if index == me {
                base_moves.push(my_move);
            } else {
                base_moves.push(simple_move(state, index));
            }
        }

        // Quem joga contra nos de verdade e todas as combinacoes de respostas.
        let minimizers = pick_minimizers(state, me);
        let mut combos: Vec<Vec<Direction>> = vec![Vec::new()];
        for index in &minimizers {
            let mut grown = Vec::new();
            for combo in &combos {
                for direction in legal_moves(state, *index) {
                    let mut extended = combo.clone();
                    extended.push(direction);
                    grown.push(extended);
                }
            }
            combos = grown;
        }

        let mut worst = INFINITY;
        for combo in combos {
            let mut moves = base_moves.clone();
            for (k, index) in minimizers.iter().enumerate() {
                moves[*index] = combo[k];
            }
            let mut next = state.clone();
            apply_turn(&mut next, &moves);
            let value = self.max_value(&next, depth - 1, alpha, beta);
            if self.out_of_time {
                return 0;
            }
            worst = worst.min(value);
            beta = beta.min(worst);
            if alpha >= beta {
                break;
            }
        }
        worst
    }
}

// ============================================================================
// ||  BLOCO: JOGADAS POSSIVEIS DE CADA COBRA
// ||  O QUE FAZ: lista as jogadas que nao batem em parede nem em corpo, e
// ||             escolhe quem joga contra nos e como as outras jogam.
// ============================================================================

/// Uma copia do tabuleiro so com o que a busca usa: corpos, vida, comida e
/// turno. Os ids viram textos curtos ("0", "1"...), mais baratos de copiar.
fn lean_copy(state: &GameState) -> GameState {
    let mut lean = state.clone();
    lean.game.ruleset.clear();
    lean.game.map = None;
    lean.game.id.clear();
    lean.board.hazards.clear();
    for (index, snake) in lean.board.snakes.iter_mut().enumerate() {
        if snake.id == state.you.id {
            lean.you.id = index.to_string();
        }
        snake.id = index.to_string();
        snake.name.clear();
        snake.latency = None;
        snake.shout = None;
        snake.extra.clear();
    }
    lean.you.name.clear();
    lean.you.latency = None;
    lean.you.shout = None;
    lean.you.extra.clear();
    lean
}

/// A casa esta livre para uma cabeca entrar no proximo turno?
// >>> So o ULTIMO pedaco de cada corpo libera a casa (a cauda anda). Se a cauda
// >>> estiver empilhada, o penultimo pedaco esta na mesma casa e a bloqueia.
fn is_free_next_turn(state: &GameState, target: Coord) -> bool {
    if target.x < 0 || target.y < 0 || target.x >= state.board.width || target.y >= state.board.height {
        return false;
    }
    for snake in &state.board.snakes {
        let last = snake.body.len().saturating_sub(1);
        for (k, segment) in snake.body.iter().enumerate() {
            if *segment == target && k != last {
                return false;
            }
        }
    }
    true
}

/// Jogadas que nao matam de imediato. Se nenhuma servir, devolve "up"
/// (a cobra morre de qualquer jeito, mas a busca precisa de uma jogada).
fn legal_moves(state: &GameState, index: usize) -> Vec<Direction> {
    let mut moves = Vec::new();
    if let Some(snake) = state.board.snakes.get(index) {
        for direction in ALL_DIRECTIONS {
            if is_free_next_turn(state, direction.step(snake.head)) {
                moves.push(direction);
            }
        }
    }
    if moves.is_empty() {
        moves.push(Direction::Up);
    }
    moves
}

/// Jogada simples para as adversarias distantes: a legal com mais saidas.
fn simple_move(state: &GameState, index: usize) -> Direction {
    let head = match state.board.snakes.get(index) {
        Some(snake) => snake.head,
        None => return Direction::Up,
    };
    let mut best = Direction::Up;
    let mut best_exits = -1;
    for direction in legal_moves(state, index) {
        let target = direction.step(head);
        let mut exits = 0;
        for next in ALL_DIRECTIONS {
            if is_free_next_turn(state, next.step(target)) {
                exits += 1;
            }
        }
        if exits > best_exits {
            best_exits = exits;
            best = direction;
        }
    }
    best
}

/// Quem joga contra nos: num duelo, a adversaria; com mais cobras, as mais
/// proximas da nossa cabeca (ate MAX_MINIMIZERS, a no maximo MINIMIZER_DISTANCE).
fn pick_minimizers(state: &GameState, me: usize) -> Vec<usize> {
    let my_head = state.board.snakes[me].head;
    let mut others: Vec<(i32, usize)> = Vec::new();
    for (index, snake) in state.board.snakes.iter().enumerate() {
        if index != me {
            others.push((manhattan(my_head, snake.head), index));
        }
    }
    if others.len() == 1 {
        return vec![others[0].1];
    }
    others.sort();
    others
        .into_iter()
        .filter(|(distance, _)| *distance <= MINIMIZER_DISTANCE)
        .take(MAX_MINIMIZERS)
        .map(|(_, index)| index)
        .collect()
}

// ============================================================================
// ||  BLOCO: TESTES DA BUSCA
// ||  O QUE FAZ: tabuleiros REAIS de derrotas na arena (06/10) e uma matada
// ||             forcada que a busca precisa enxergar.
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Battlesnake, Board, Game};
    use std::collections::HashMap;
    use std::time::Duration;

    fn snake(id: &str, cells: &[(i32, i32)], health: i32) -> Battlesnake {
        let body: Vec<Coord> = cells.iter().map(|(x, y)| Coord { x: *x, y: *y }).collect();
        Battlesnake {
            id: id.to_string(),
            name: id.to_string(),
            health,
            head: body[0],
            length: body.len() as i32,
            body,
            latency: None,
            shout: None,
            extra: Default::default(),
        }
    }

    fn state(turn: i32, me: Battlesnake, others: Vec<Battlesnake>, food: &[(i32, i32)]) -> GameState {
        let mut snakes = vec![me.clone()];
        snakes.extend(others);
        GameState {
            game: Game { id: "busca".to_string(), ruleset: HashMap::new(), map: None, timeout: 500 },
            turn,
            board: Board {
                width: 11,
                height: 11,
                food: food.iter().map(|(x, y)| Coord { x: *x, y: *y }).collect(),
                hazards: vec![],
                snakes,
            },
            you: me,
        }
    }

    /// A decisao completa (rede + busca) com 200 ms para pensar.
    fn decide(s: &GameState) -> Direction {
        crate::logic::decide(s, Instant::now() + Duration::from_millis(200)).0
    }

    #[test]
    fn nao_desce_para_a_linha_de_baixo_entre_duas_cobras() {
        // Partida 8a71dd92 (4 cobras), turno 10. A v0510 desceu para (7,0) e,
        // no turno seguinte, so tinha uma cabeca com cabeca perdida.
        //
        // y=3  . . . . . . e b . . .
        // y=2  . . . . . . T b . . .     T = Tokuji_typescript (tamanho 5)
        // y=1  . . . . . . . H . . .     H = nos (tamanho 4)
        // y=0  . . . . . . F . L l l     L = Lkobra_2 (tamanho 3)
        //      0 1 2 3 4 5 6 7 8 9 10
        let me = snake("eu", &[(7, 1), (7, 2), (7, 3), (7, 4)], 92);
        let typescript = snake("ts", &[(6, 2), (6, 3), (6, 4), (6, 5), (6, 6)], 93);
        let java = snake("java", &[(1, 9), (1, 10), (0, 10), (0, 9)], 92);
        let lkobra = snake("lk", &[(8, 0), (9, 0), (10, 0)], 90);
        let s = state(10, me, vec![typescript, java, lkobra], &[(6, 0), (5, 5)]);
        assert_ne!(decide(&s), Direction::Down);
    }

    #[test]
    fn nao_entra_no_corredor_de_cima_contra_tokuji_rust() {
        // Partida 522f65c8 (duelo), turno 63. A v0510 seguiu para a direita pela
        // linha de cima; dali em diante nao havia volta: a Tokuji_rust fechou a
        // saida e nos batemos no corpo dela no turno 67. A saida certa era
        // descer pela coluna x=5, que a cauda dela vai liberando.
        //
        // y=10  . . . . b H . . . . .
        // y= 9  . . . . b . r r E . .
        // y= 8  . . . b b . r . . . .
        let me = snake(
            "eu",
            &[(5, 10), (4, 10), (4, 9), (4, 8), (3, 8), (3, 7), (3, 6), (3, 5), (2, 5), (2, 6)],
            85,
        );
        let rust = snake(
            "rust",
            &[(8, 9), (7, 9), (6, 9), (6, 8), (6, 7), (6, 6), (6, 5), (6, 4), (5, 4), (4, 4)],
            93,
        );
        let s = state(63, me, vec![rust], &[(9, 9), (7, 5)]);
        assert_eq!(decide(&s), Direction::Down);
    }

    #[test]
    fn enxerga_a_matada_forcada_no_corredor() {
        // A adversaria (tamanho 5) esta no corredor entre a parede de baixo e o
        // nosso corpo. Fechando a saida, ela morre em poucos turnos, nao importa
        // o que faca: a busca tem que encontrar isso (nota de vitoria).
        //
        // y=2  . . . . . . . .
        // y=1  . H b b b b b t
        // y=0  . . . E e e e e
        //      0 1 2 3 4 5 6 7
        let me = snake("eu", &[(1, 1), (2, 1), (3, 1), (4, 1), (5, 1), (6, 1), (7, 1)], 100);
        let enemy = snake("ela", &[(3, 0), (4, 0), (5, 0), (6, 0), (7, 0)], 100);
        let s = state(20, me, vec![enemy], &[]);
        let ranking = crate::safety::choose_move(&s).ranking;
        let result = search(&s, Instant::now() + Duration::from_millis(500), &ranking, &crate::eval::EVAL).unwrap();
        assert!(result.score >= WIN / 2, "nota {} em profundidade {}", result.score, result.depth);
    }

    #[test]
    fn busca_respeita_o_prazo() {
        // Mesmo num tabuleiro cheio de opcoes, a busca devolve logo depois do prazo.
        let me = snake("eu", &[(5, 5), (5, 4), (5, 3)], 100);
        let a = snake("a", &[(1, 1), (1, 2), (1, 3)], 100);
        let b = snake("b", &[(9, 9), (9, 8), (9, 7)], 100);
        let c = snake("c", &[(1, 9), (2, 9), (3, 9)], 100);
        let s = state(5, me, vec![a, b, c], &[(5, 7), (2, 2)]);
        let started = Instant::now();
        crate::logic::decide(&s, started + Duration::from_millis(50));
        assert!(started.elapsed() < Duration::from_millis(80), "levou {:?}", started.elapsed());
    }
}

#[cfg(test)]
mod speed {
    use super::*;
    use crate::models::{Battlesnake, Board, Game};
    use serde_json::json;
    use std::collections::HashMap;
    use std::time::Duration;

    /// Quantos tabuleiros a busca olha em 200 ms num duelo de meio de partida,
    /// com o pacote de regras do jeito que a arena manda (rode com --release).
    #[test]
    #[ignore]
    fn velocidade_da_busca() {
        let body = |cells: &[(i32, i32)]| -> Vec<Coord> { cells.iter().map(|(x, y)| Coord { x: *x, y: *y }).collect() };
        let snake = |id: &str, cells: &[(i32, i32)]| Battlesnake {
            id: id.to_string(),
            name: format!("cobra {id}"),
            health: 80,
            head: body(cells)[0],
            length: cells.len() as i32,
            body: body(cells),
            latency: Some("123".to_string()),
            shout: Some("uma frase qualquer de exemplo".to_string()),
            extra: Default::default(),
        };
        let me = snake("6b6886f0-1234-4321-abcd-0123456789ab", &[(5, 5), (5, 4), (5, 3), (4, 3), (3, 3), (3, 4), (3, 5), (3, 6), (3, 7), (3, 8)]);
        let enemy = snake("b85cd55e-1234-4321-abcd-0123456789ab", &[(8, 6), (8, 5), (8, 4), (9, 4), (9, 3), (9, 2), (8, 2), (7, 2)]);
        let mut ruleset = HashMap::new();
        ruleset.insert("name".to_string(), json!("standard"));
        ruleset.insert("version".to_string(), json!("v1.2.3"));
        ruleset.insert("settings".to_string(), json!({"foodSpawnChance": 15, "minimumFood": 1, "hazardDamagePerTurn": 14,
            "royale": {"shrinkEveryNTurns": 25}, "squad": {"allowBodyCollisions": false, "sharedElimination": false}}));
        let state = GameState {
            game: Game { id: "7d1c2c1e-1234-4321-abcd-0123456789ab".to_string(), ruleset, map: Some("standard".to_string()), timeout: 500 },
            turn: 50,
            board: Board { width: 11, height: 11, food: body(&[(1, 1), (9, 9), (6, 8)]), hazards: vec![], snakes: vec![me.clone(), enemy] },
            you: me,
        };
        let ranking = crate::safety::choose_move(&state).ranking;
        let result = search(&state, Instant::now() + Duration::from_millis(200), &ranking, &crate::eval::EVAL).unwrap();
        println!("\n200 ms: {} tabuleiros, profundidade {}", result.nodes, result.depth);

    }
}
