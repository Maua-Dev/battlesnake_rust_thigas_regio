// ============================================================================
// ||  BLOCO: PESOS DA AVALIACAO
// ||  O QUE FAZ: os numeros que dizem quanto vale cada coisa num tabuleiro.
// ||  POR QUE:   a busca imagina varios futuros e precisa dar uma nota a cada
// ||             um para escolher o melhor.
// ============================================================================

/// Nota de vitoria (todas as adversarias morreram e nos estamos vivas).
pub const WIN: i64 = 1_000_000_000;
/// Todas as cobras morreram no mesmo turno: ruim, mas melhor que perder sozinha.
const DRAW: i64 = -500_000_000;
/// Ganhar mais cedo vale mais; perder mais tarde e menos ruim.
const TURN_VALUE: i64 = 1_000;

/// Ficar num espaco menor que o proprio corpo (beco).
const TRAPPED_PENALTY: i64 = 50_000_000;
/// A comida mais perto esta mais longe do que a vida que nos resta.
const STARVING_PENALTY: i64 = 40_000_000;
/// Depois de tantas unidades a mais que a maior adversaria, crescer nao conta mais.
const MAX_USEFUL_LEAD: i64 = 4;
/// Sem comida alcancavel, conta como se ela estivesse a tantos passos.
const FOOD_HORIZON: i64 = 30;
const HUNGRY_HEALTH: i32 = 50;
const WANTED_LENGTH_LEAD: i32 = 3;
/// A urgencia da caca sobe 1 ponto a cada tantos turnos (1 no inicio)...
const TURNS_PER_URGENCY_STEP: i32 = 33;
/// ...ate este teto (alcancado no turno 99). Caçar mais forte que isso piora.
const MAX_URGENCY: i64 = 4;

/// Os pesos que o self-play ajusta. `Copy` deixa passar a struct por valor.
#[derive(Debug, Clone, Copy)]
pub struct EvalWeights {
    /// Etiqueta para os relatorios do self-play.
    pub tag: &'static str,
    /// Cada casa de territorio (Voronoi).
    pub territory: i64,
    /// Cada unidade de tamanho a mais que a maior adversaria (ate MAX_USEFUL_LEAD).
    pub length: i64,
    /// Com fome (ou sem a vantagem de tamanho desejada): cada passo ate a comida.
    pub food: i64,
    /// Sendo a maior: cada passo ate a cabeca adversaria mais proxima, vezes a urgencia.
    pub hunt: i64,
    /// Sendo a maior: cada casa em que a adversaria ainda consegue andar, vezes
    /// a urgencia. E o "aperto": quanto menos espaco ela tiver, melhor.
    pub squeeze: i64,
    /// Cada adversaria presa num espaco menor que o corpo dela.
    pub enemy_trapped: i64,
}

/// Os pesos que a cobra usa na arena.
pub const EVAL: EvalWeights = EvalWeights {
    tag: "b0810",
    territory: 10,
    length: 2_000,
    food: 50,
    hunt: 20,
    squeeze: 0,
    enemy_trapped: 5_000_000,
};

use crate::board::{count_reachable, distances_from, manhattan, territory, Grid, UNREACHABLE};
use crate::models::GameState;

// ============================================================================
// ||  BLOCO: FIM DE JOGO
// ||  O QUE FAZ: reconhece tabuleiros onde a partida ja acabou.
// ||  POR QUE:   nesses casos a nota e exata (vitoria, derrota, empate) e a
// ||             busca nao precisa olhar mais fundo.
// ============================================================================

/// A nota exata se a partida acabou neste tabuleiro; `None` se ela continua.
pub fn terminal_value(state: &GameState, me_id: &str) -> Option<i64> {
    let me_alive = state.board.snakes.iter().any(|s| s.id == me_id);
    let others_alive = state.board.snakes.iter().filter(|s| s.id != me_id).count();
    let turn = state.turn as i64;

    if !me_alive {
        if others_alive == 0 {
            return Some(DRAW + turn * TURN_VALUE);
        }
        return Some(-WIN + turn * TURN_VALUE);
    }
    if others_alive == 0 {
        return Some(WIN - turn * TURN_VALUE);
    }
    None
}

// ============================================================================
// ||  BLOCO: NOTA DE UM TABULEIRO EM ANDAMENTO
// ||  O QUE FAZ: soma territorio, tamanho, comida, caca, aperto e becos
// ||             (nossos e das adversarias), do ponto de vista da cobra `me_id`.
// ============================================================================

pub fn evaluate(state: &GameState, me_id: &str, weights: &EvalWeights) -> i64 {
    if let Some(value) = terminal_value(state, me_id) {
        return value;
    }
    let me = match state.board.snakes.iter().find(|s| s.id == me_id) {
        Some(snake) => snake,
        None => return -WIN,
    };
    let grid = match Grid::from_state(state) {
        Some(grid) => grid,
        None => return 0,
    };

    let my_length = me.body.len() as i32;
    let mut longest_enemy = 0;
    let mut enemy_heads = Vec::new();
    for snake in &state.board.snakes {
        if snake.id != me_id {
            longest_enemy = longest_enemy.max(snake.body.len() as i32);
            enemy_heads.push(snake.head);
        }
    }

    let mine = distances_from(&grid, &[me.head], 0, 0);
    let theirs = distances_from(&grid, &enemy_heads, 0, 0);
    let lead = (my_length - longest_enemy) as i64;
    let urgency = (1 + (state.turn.max(0) / TURNS_PER_URGENCY_STEP) as i64).min(MAX_URGENCY);
    let mut score: i64 = 0;

    // Becos: o nosso (muito ruim) e o das adversarias (muito bom).
    // >>> -1 porque a contagem inclui a casa da propria cabeca.
    if count_reachable(&mine) - 1 < my_length {
        score -= TRAPPED_PENALTY;
    }
    let mut enemy_space_total = 0;
    for snake in &state.board.snakes {
        if snake.id == me_id {
            continue;
        }
        // >>> Num duelo, a BFS "de todas as adversarias" ja e a BFS dela:
        // >>> reaproveitamos em vez de fazer outra (a busca fica mais rapida).
        let space = if enemy_heads.len() == 1 {
            count_reachable(&theirs) - 1
        } else {
            count_reachable(&distances_from(&grid, &[snake.head], 0, 0)) - 1
        };
        enemy_space_total += space;
        if space < snake.body.len() as i32 {
            score += weights.enemy_trapped;
        }
    }

    // Territorio: em empate de distancia, a casa e de quem for maior.
    score += territory(&mine, &theirs, my_length > longest_enemy) as i64 * weights.territory;

    // Tamanho: ser maior ganha cabeca com cabeca. Depois de 4 a mais, tanto faz.
    score += lead.min(MAX_USEFUL_LEAD) * weights.length;

    // Comida: a mais perto que alcancamos.
    let mut nearest_food: Option<u32> = None;
    for food in &state.board.food {
        if let Some(index) = grid.index_of(*food) {
            let distance = mine[index];
            if distance != UNREACHABLE && nearest_food.map_or(true, |best| distance < best) {
                nearest_food = Some(distance);
            }
        }
    }
    let hungry = me.health < HUNGRY_HEALTH || (lead as i32) < WANTED_LENGTH_LEAD;
    match nearest_food {
        Some(distance) => {
            if me.health <= distance as i32 {
                score -= STARVING_PENALTY;
            }
            if hungry {
                score -= distance as i64 * weights.food;
            }
        }
        None => {
            if hungry {
                score -= FOOD_HORIZON * weights.food;
            }
        }
    }

    // Sendo a maior: cacar (chegar perto da cabeca) e apertar (tirar espaco
    // dela). As duas coisas pesam mais com o passar dos turnos (ate o teto).
    if lead > 0 {
        let mut nearest = i32::MAX;
        for head in &enemy_heads {
            nearest = nearest.min(manhattan(me.head, *head));
        }
        score -= nearest as i64 * weights.hunt * urgency;
        score -= enemy_space_total as i64 * weights.squeeze * urgency;
    }

    score
}
