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
/// Cada adversaria presa num espaco menor que o corpo dela.
const ENEMY_TRAPPED_BONUS: i64 = 5_000_000;
/// A comida mais perto esta mais longe do que a vida que nos resta.
const STARVING_PENALTY: i64 = 40_000_000;

/// Cada casa de territorio (Voronoi).
const TERRITORY_WEIGHT: i64 = 10;
/// Cada unidade de tamanho a mais que a maior adversaria (ate MAX_USEFUL_LEAD).
const LENGTH_WEIGHT: i64 = 2_000;
const MAX_USEFUL_LEAD: i64 = 4;
/// Com fome (ou sem a vantagem de tamanho desejada): cada passo ate a comida.
const FOOD_WEIGHT: i64 = 50;
const FOOD_HORIZON: i64 = 30;
const HUNGRY_HEALTH: i32 = 50;
const WANTED_LENGTH_LEAD: i32 = 3;
/// Sendo a maior: cada passo ate a cabeca adversaria mais proxima, vezes a urgencia.
const HUNT_WEIGHT: i64 = 20;
const TURNS_PER_URGENCY_STEP: i32 = 33;

use crate::board::{count_reachable, distances_from, manhattan, territory, Grid, UNREACHABLE};
use crate::models::GameState;
use crate::rules::ARENA_LAST_TURN;

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
    // >>> No turno 99 a arena encerra a partida e, com mais de uma viva, o
    // >>> desempate parece sorteio. Chegar la vale "meio a meio": nota neutra.
    if state.turn >= ARENA_LAST_TURN {
        return Some(0);
    }
    None
}

// ============================================================================
// ||  BLOCO: NOTA DE UM TABULEIRO EM ANDAMENTO
// ||  O QUE FAZ: soma territorio, tamanho, comida, caca e becos (nossos e
// ||             das adversarias), do ponto de vista da cobra `me_id`.
// ============================================================================

pub fn evaluate(state: &GameState, me_id: &str) -> i64 {
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
    let mut score: i64 = 0;

    // Becos: o nosso (muito ruim) e o das adversarias (muito bom).
    // >>> -1 porque a contagem inclui a casa da propria cabeca.
    if count_reachable(&mine) - 1 < my_length {
        score -= TRAPPED_PENALTY;
    }
    for snake in &state.board.snakes {
        if snake.id == me_id {
            continue;
        }
        let reach = distances_from(&grid, &[snake.head], 0, 0);
        if count_reachable(&reach) - 1 < snake.body.len() as i32 {
            score += ENEMY_TRAPPED_BONUS;
        }
    }

    // Territorio: em empate de distancia, a casa e de quem for maior.
    score += territory(&mine, &theirs, my_length > longest_enemy) as i64 * TERRITORY_WEIGHT;

    // Tamanho: ser maior ganha cabeca com cabeca. Depois de 4 a mais, tanto faz.
    let lead = (my_length - longest_enemy) as i64;
    score += lead.min(MAX_USEFUL_LEAD) * LENGTH_WEIGHT;

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
                score -= distance as i64 * FOOD_WEIGHT;
            }
        }
        None => {
            if hungry {
                score -= FOOD_HORIZON * FOOD_WEIGHT;
            }
        }
    }

    // Caca: sendo a maior, chegar perto da cabeca adversaria, mais ainda no fim.
    if lead > 0 {
        let mut nearest = i32::MAX;
        for head in &enemy_heads {
            nearest = nearest.min(manhattan(me.head, *head));
        }
        let urgency = 1 + (state.turn.max(0) / TURNS_PER_URGENCY_STEP) as i64;
        score -= nearest as i64 * HUNT_WEIGHT * urgency;
    }

    score
}
