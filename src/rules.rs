// ============================================================================
// ||  BLOCO: SIMULADOR DAS REGRAS (MODO STANDARD)
// ||  O QUE FAZ: aplica um turno inteiro: todas as cobras andam, comem e
// ||             as eliminacoes acontecem, na ordem oficial do jogo.
// ||  POR QUE:   para jogar partidas inteiras aqui no computador (self-play)
// ||             e, depois, para a cobra "imaginar" turnos futuros na busca.
// ||  FONTE:     https://docs.battlesnake.com/rules
// ============================================================================

use crate::board::Direction;
use crate::models::{Coord, GameState};

// >>> A arena NAO encerra no turno 99. A API de quadros devolve no maximo 100
// >>> por pedido (descoberto em 10/10): as partidas seguem ate sobrar uma cobra.
// >>> A arena oficial so para no turno 5000.

/// Uma cobra que saiu do jogo neste turno e o motivo (nomes iguais aos da arena).
#[derive(Debug, Clone, PartialEq)]
pub struct Elimination {
    pub id: String,
    pub cause: &'static str,
}

/// Aplica um turno completo. `moves[i]` e a direcao da cobra `board.snakes[i]`.
/// Devolve quem foi eliminado. As eliminadas saem de `board.snakes`.
pub fn apply_turn(state: &mut GameState, moves: &[Direction]) -> Vec<Elimination> {
    let width = state.board.width;
    let height = state.board.height;

    // 1. Todas as cobras andam AO MESMO TEMPO: cabeca nova na frente,
    //    a cauda sai, e a vida cai 1.
    for (i, snake) in state.board.snakes.iter_mut().enumerate() {
        let direction = moves.get(i).copied().unwrap_or(Direction::Up);
        let new_head = direction.step(snake.head);
        snake.body.insert(0, new_head);
        snake.body.pop();
        snake.head = new_head;
        snake.health -= 1;
    }

    // 2. Quem esta em cima de comida: vida volta a 100 e o corpo cresce 1.
    // >>> O crescimento repete o ultimo pedaco (a cauda fica "empilhada").
    // >>> Se duas cobras chegam juntas na mesma comida, as duas comem.
    let mut eaten: Vec<Coord> = Vec::new();
    for snake in state.board.snakes.iter_mut() {
        if state.board.food.contains(&snake.head) {
            snake.health = 100;
            if let Some(tail) = snake.body.last().copied() {
                snake.body.push(tail);
            }
            eaten.push(snake.head);
        }
    }
    state.board.food.retain(|food| !eaten.contains(food));

    // 3a. Eliminacoes que nao dependem das outras: fome e parede.
    let count = state.board.snakes.len();
    let mut cause: Vec<Option<&'static str>> = vec![None; count];
    for (i, snake) in state.board.snakes.iter().enumerate() {
        let h = snake.head;
        if snake.health <= 0 {
            cause[i] = Some("out-of-health");
        } else if h.x < 0 || h.y < 0 || h.x >= width || h.y >= height {
            cause[i] = Some("wall-collision");
        }
    }

    // 3b. Colisoes, so entre as que sobraram do passo 3a. Todas sao
    //     decididas ao mesmo tempo, olhando o tabuleiro depois do movimento.
    let mut collision: Vec<Option<&'static str>> = vec![None; count];
    for i in 0..count {
        if cause[i].is_some() {
            continue;
        }
        let head = state.board.snakes[i].head;
        let my_length = state.board.snakes[i].body.len();
        for j in 0..count {
            if cause[j].is_some() {
                continue;
            }
            let other = &state.board.snakes[j];
            // >>> Bater no CORPO (tudo menos a cabeca) de qualquer cobra, inclusive a propria.
            if other.body.iter().skip(1).any(|segment| *segment == head) {
                collision[i] = Some(if i == j { "snake-self-collision" } else { "snake-collision" });
                break;
            }
            // >>> Cabeca com cabeca: a menor morre; tamanhos iguais, morrem as duas.
            if i != j && other.head == head && my_length <= other.body.len() {
                collision[i] = Some("head-collision");
                break;
            }
        }
    }

    // Junta as eliminacoes e tira as cobras eliminadas do tabuleiro.
    let mut eliminated = Vec::new();
    for i in 0..count {
        if let Some(reason) = cause[i].or(collision[i]) {
            eliminated.push(Elimination {
                id: state.board.snakes[i].id.clone(),
                cause: reason,
            });
        }
    }
    state
        .board
        .snakes
        .retain(|snake| !eliminated.iter().any(|e| e.id == snake.id));

    for snake in state.board.snakes.iter_mut() {
        snake.length = snake.body.len() as i32;
    }
    state.turn += 1;
    eliminated
}

// ============================================================================
// ||  BLOCO: TESTES DE REGRAS
// ||  O QUE FAZ: um teste por regra: andar, comer, cauda empilhada, parede,
// ||             fome, corpo, e cabeca com cabeca (maior, menor, igual).
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::Direction::{Down, Left, Right, Up};
    use crate::models::{Battlesnake, Board, Game};
    use std::collections::HashMap;

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

    fn state(snakes: Vec<Battlesnake>, food: &[(i32, i32)]) -> GameState {
        GameState {
            game: Game { id: "r".to_string(), ruleset: HashMap::new(), map: None, timeout: 500 },
            turn: 5,
            board: Board {
                width: 11,
                height: 11,
                food: food.iter().map(|(x, y)| Coord { x: *x, y: *y }).collect(),
                hazards: vec![],
                snakes: snakes.clone(),
            },
            you: snakes[0].clone(),
        }
    }

    fn c(x: i32, y: i32) -> Coord {
        Coord { x, y }
    }

    #[test]
    fn andar_move_cabeca_tira_cauda_e_gasta_vida() {
        let mut s = state(vec![snake("a", &[(5, 5), (5, 4), (5, 3)], 50)], &[]);
        let dead = apply_turn(&mut s, &[Up]);
        assert!(dead.is_empty());
        let a = &s.board.snakes[0];
        assert_eq!(a.body, vec![c(5, 6), c(5, 5), c(5, 4)]);
        assert_eq!(a.health, 49);
        assert_eq!(s.turn, 6);
    }

    #[test]
    fn comer_enche_a_vida_cresce_e_some_a_comida() {
        let mut s = state(vec![snake("a", &[(5, 5), (5, 4), (5, 3)], 50)], &[(5, 6), (0, 0)]);
        apply_turn(&mut s, &[Up]);
        let a = &s.board.snakes[0];
        assert_eq!(a.health, 100);
        assert_eq!(a.body, vec![c(5, 6), c(5, 5), c(5, 4), c(5, 4)]);
        assert_eq!(s.board.food, vec![c(0, 0)]);
    }

    #[test]
    fn cauda_empilhada_continua_no_lugar_um_turno() {
        // Comeu no turno passado: (5,3) aparece duas vezes.
        let mut s = state(vec![snake("a", &[(5, 5), (5, 4), (5, 3), (5, 3)], 90)], &[]);
        apply_turn(&mut s, &[Right]);
        assert_eq!(s.board.snakes[0].body, vec![c(6, 5), c(5, 5), c(5, 4), c(5, 3)]);
    }

    #[test]
    fn pode_entrar_na_casa_onde_estava_a_propria_cauda() {
        let mut s = state(vec![snake("a", &[(0, 0), (0, 1), (1, 1), (1, 0)], 90)], &[]);
        let dead = apply_turn(&mut s, &[Right]);
        assert!(dead.is_empty());
    }

    #[test]
    fn sair_do_tabuleiro_elimina() {
        let mut s = state(vec![snake("a", &[(0, 5), (1, 5), (2, 5)], 90)], &[]);
        let dead = apply_turn(&mut s, &[Left]);
        assert_eq!(dead[0].cause, "wall-collision");
        assert!(s.board.snakes.is_empty());
    }

    #[test]
    fn vida_zerada_elimina_mas_comer_no_ultimo_turno_salva() {
        let mut s = state(
            vec![snake("a", &[(5, 5), (5, 4), (5, 3)], 1), snake("b", &[(8, 5), (8, 4), (8, 3)], 1)],
            &[(8, 6)],
        );
        let dead = apply_turn(&mut s, &[Up, Up]);
        assert_eq!(dead, vec![Elimination { id: "a".to_string(), cause: "out-of-health" }]);
        assert_eq!(s.board.snakes[0].id, "b");
    }

    #[test]
    fn bater_no_proprio_corpo_elimina() {
        let mut s = state(vec![snake("a", &[(5, 5), (5, 4), (6, 4), (6, 5), (6, 6)], 90)], &[]);
        let dead = apply_turn(&mut s, &[Right]);
        assert_eq!(dead[0].cause, "snake-self-collision");
    }

    #[test]
    fn bater_no_corpo_de_outra_elimina_so_quem_bateu() {
        let mut s = state(
            vec![snake("a", &[(5, 5), (5, 4), (5, 3)], 90), snake("b", &[(6, 7), (5, 7), (4, 7), (3, 7)], 90)],
            &[],
        );
        // a sobe para (5,6); b anda para a direita e o corpo dela passa por (5,7)... a nao bate.
        // Agora a sobe de novo para (5,7), onde esta o corpo de b.
        apply_turn(&mut s, &[Up, Right]);
        let dead = apply_turn(&mut s, &[Up, Right]);
        assert_eq!(dead, vec![Elimination { id: "a".to_string(), cause: "snake-collision" }]);
    }

    #[test]
    fn cabeca_com_cabeca_maior_vence_menor_morre() {
        let mut s = state(
            vec![snake("a", &[(5, 5), (5, 4), (5, 3), (5, 2)], 90), snake("b", &[(5, 7), (5, 8), (5, 9)], 90)],
            &[],
        );
        let dead = apply_turn(&mut s, &[Up, Down]);
        assert_eq!(dead, vec![Elimination { id: "b".to_string(), cause: "head-collision" }]);
        assert_eq!(s.board.snakes.len(), 1);
    }

    #[test]
    fn cabeca_com_cabeca_tamanhos_iguais_morrem_as_duas() {
        let mut s = state(
            vec![snake("a", &[(5, 5), (5, 4), (5, 3)], 90), snake("b", &[(5, 7), (5, 8), (5, 9)], 90)],
            &[],
        );
        let dead = apply_turn(&mut s, &[Up, Down]);
        assert_eq!(dead.len(), 2);
        assert!(s.board.snakes.is_empty());
    }

    #[test]
    fn cabeca_com_cabeca_em_cima_de_comida_as_duas_crescem_antes() {
        // As duas comem a mesma comida e crescem 1: a maior continua maior.
        let mut s = state(
            vec![snake("a", &[(5, 5), (5, 4), (5, 3), (5, 2)], 90), snake("b", &[(5, 7), (5, 8), (5, 9)], 90)],
            &[(5, 6)],
        );
        let dead = apply_turn(&mut s, &[Up, Down]);
        assert_eq!(dead, vec![Elimination { id: "b".to_string(), cause: "head-collision" }]);
        assert!(s.board.food.is_empty());
    }
}
