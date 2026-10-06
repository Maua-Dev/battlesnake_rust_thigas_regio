// ============================================================================
// ||  BLOCO: CONSTANTES DE AJUSTE
// ||  O QUE FAZ: todos os "pesos" da decisao num lugar so.
// ||  POR QUE:   para ajustar o jeito da cobra sem cacar numeros no codigo.
// ============================================================================

/// Abaixo desta vida a cobra passa a procurar comida.
const HUNGRY_HEALTH: i32 = 50;
/// Abaixo desta vida comer vale mais do que espaco (mas nunca mais que seguranca).
const CRITICAL_HEALTH: i32 = 25;
/// Queremos ser pelo menos esta quantidade maior que a maior adversaria.
const WANTED_LENGTH_LEAD: i32 = 2;

/// Espaco "confortavel" = comprimento * MULTIPLIER + EXTRA. Acima disso,
/// mais espaco nao muda a nota (quem decide e a comida ou o centro).
const COMFORT_MULTIPLIER: i32 = 2;
const COMFORT_EXTRA: i32 = 4;

/// Notas de cada camada. Cada camada vale mais que tudo das camadas de baixo.
const SCORE_NOT_TRAPPED: i64 = 1_000_000;
const SCORE_NO_HEAD_RISK: i64 = 100_000;
const SCORE_PER_SPACE_CELL: i64 = 100;
/// Comida: a nota cresce quanto mais perto, ate este horizonte de passos.
const FOOD_HORIZON: i64 = 30;
const FOOD_WEIGHT: i64 = 3;
const FOOD_WEIGHT_CRITICAL: i64 = 1_000;
/// Bonus por mirar a casa onde uma cobra MENOR pode colocar a cabeca.
const KILL_BONUS: i64 = 60;
/// Desempate final: cada passo mais perto do centro vale isto.
const CENTER_WEIGHT: i64 = 1;

/// Etiqueta curta no `shout`, para confirmar nos replays qual versao jogou.
const VERSION_TAG: &str = "v0510";

use crate::board::{Direction, Grid, ALL_DIRECTIONS};
use crate::models::{Coord, GameState};
use std::collections::VecDeque;

/// Distancia de uma casa que nao da para alcancar.
const UNREACHABLE: u32 = u32::MAX;

// ============================================================================
// ||  BLOCO: ANALISE DE UMA JOGADA
// ||  O QUE FAZ: guarda tudo o que descobrimos sobre ir para uma direcao.
// ||  POR QUE:   a escolha final compara as 4 direcoes por estes numeros.
// ============================================================================

/// O que sabemos sobre uma das 4 direcoes. Uma `struct` junta varios
/// valores com nome numa coisa so.
#[derive(Debug, Clone)]
pub struct MoveInfo {
    pub direction: Direction,
    /// A casa onde a nossa cabeca vai parar.
    pub target: Coord,
    /// Nao bate em parede nem em corpo no proximo turno.
    pub legal: bool,
    /// Quantas casas a cabeca consegue alcancar depois desta jogada.
    pub space: i32,
    /// O espaco e menor que o nosso comprimento: provavelmente um beco.
    pub trapped: bool,
    /// Uma cobra maior ou igual pode colocar a cabeca na mesma casa.
    pub head_risk: bool,
    /// Uma cobra menor pode colocar a cabeca na mesma casa (e morreria).
    pub kill_chance: bool,
    /// Esta jogada come uma comida.
    pub eats: bool,
    /// Passos ate a comida mais proxima (preferindo as que chegamos primeiro).
    pub food_distance: Option<u32>,
    pub score: i64,
}

/// A decisao final: a direcao e o texto curto para o `shout`.
pub struct Decision {
    pub direction: Direction,
    pub shout: String,
}

// ============================================================================
// ||  BLOCO: ESCOLHA DA JOGADA
// ||  O QUE FAZ: analisa as 4 direcoes e escolhe a de maior nota.
// ||  POR QUE:   e a "rede de seguranca": nunca escolhe uma jogada que mata
// ||             agora se existir outra, e foge de becos e de cobras maiores.
// ============================================================================

pub fn choose_move(state: &GameState) -> Decision {
    let infos = analyze_moves(state);

    let mut best: Option<&MoveInfo> = None;
    for info in &infos {
        if !info.legal {
            continue;
        }
        let is_better = match best {
            None => true,
            Some(current) => info.score > current.score,
        };
        if is_better {
            best = Some(info);
        }
    }

    match best {
        Some(info) => Decision {
            direction: info.direction,
            shout: describe(info),
        },
        // >>> Nenhuma direcao legal: qualquer jogada morre. Respondemos algo valido.
        None => Decision {
            direction: emergency_move(state),
            shout: format!("{} sem saida", VERSION_TAG),
        },
    }
}

/// Analisa as 4 direcoes. Devolve uma lista vazia se o tabuleiro for invalido.
pub fn analyze_moves(state: &GameState) -> Vec<MoveInfo> {
    let mut infos = Vec::new();
    let grid = match Grid::from_state(state) {
        Some(grid) => grid,
        None => return infos,
    };

    let me = &state.you;
    let my_head = me.head;
    let my_length = me.body.len() as i32;

    // Separa as adversarias: as maiores ou iguais sao perigo; as menores, alvo.
    let mut big_heads: Vec<Coord> = Vec::new();
    let mut small_heads: Vec<Coord> = Vec::new();
    let mut longest_enemy = 0;
    for snake in &state.board.snakes {
        if snake.id == me.id {
            continue;
        }
        let length = snake.body.len() as i32;
        if length > longest_enemy {
            longest_enemy = length;
        }
        if length >= my_length {
            big_heads.push(snake.head);
        } else {
            small_heads.push(snake.head);
        }
    }

    // >>> Distancia de cada casa ate a cabeca maior-ou-igual mais proxima.
    // >>> Serve para saber quais comidas conseguimos pegar ANTES delas.
    let enemy_distance = distances_from(&grid, &big_heads, 0, 0);

    let wants_food =
        me.health < HUNGRY_HEALTH || my_length < longest_enemy + WANTED_LENGTH_LEAD;
    let critical = me.health < CRITICAL_HEALTH;

    for direction in ALL_DIRECTIONS {
        let target = direction.step(my_head);
        let mut info = MoveInfo {
            direction,
            target,
            legal: grid.can_enter(target, 1, 0),
            space: 0,
            trapped: true,
            head_risk: is_next_to_any(target, &big_heads),
            kill_chance: is_next_to_any(target, &small_heads),
            eats: state.board.food.contains(&target),
            food_distance: None,
            score: 0,
        };

        if info.legal {
            // >>> Se esta jogada come, a nossa cauda fica parada 1 turno:
            // >>> todas as casas do nosso corpo demoram 1 turno a mais para liberar.
            let my_delay = if info.eats { 1 } else { 0 };
            let my_distance = distances_from(&grid, &[target], 1, my_delay);

            info.space = my_distance.iter().filter(|d| **d != UNREACHABLE).count() as i32;
            let length_after = my_length + if info.eats { 1 } else { 0 };
            info.trapped = info.space < length_after;
            info.food_distance = nearest_food(&grid, state, &my_distance, &enemy_distance);
            info.score = score_move(&info, &grid, my_length, wants_food, critical);
        }

        infos.push(info);
    }

    infos
}

/// A nota de uma jogada legal, em camadas: cada camada vale mais que
/// tudo o que vem depois dela.
fn score_move(info: &MoveInfo, grid: &Grid, my_length: i32, wants_food: bool, critical: bool) -> i64 {
    let mut score = 0;

    // Camada 1: nao entrar em beco.
    if !info.trapped {
        score += SCORE_NOT_TRAPPED;
    }
    // Camada 2: nao arriscar cabeca com cabeca contra cobra maior ou igual.
    if !info.head_risk {
        score += SCORE_NO_HEAD_RISK;
    }
    // Camada 3: espaco, ate o limite do confortavel.
    let comfortable = my_length * COMFORT_MULTIPLIER + COMFORT_EXTRA;
    score += info.space.min(comfortable) as i64 * SCORE_PER_SPACE_CELL;

    // Camada 4: comida, se quisermos comer. Com vida critica, pesa bem mais.
    if wants_food || critical {
        if let Some(distance) = info.food_distance {
            let closeness = (FOOD_HORIZON - distance as i64).max(0);
            let weight = if critical { FOOD_WEIGHT_CRITICAL } else { FOOD_WEIGHT };
            score += closeness * weight;
        }
    }

    // Desempates: atacar cobra menor e ficar perto do centro.
    if info.kill_chance && !info.head_risk {
        score += KILL_BONUS;
    }
    score -= distance_to_center(grid, info.target) * CENTER_WEIGHT;

    score
}

// ============================================================================
// ||  BLOCO: BUSCA EM LARGURA (BFS) QUE SABE QUE AS CAUDAS ANDAM
// ||  O QUE FAZ: calcula em quantos turnos chegamos a cada casa.
// ||  POR QUE:   com isso medimos o espaco (flood fill) e a distancia ate
// ||             a comida, numa passada so.
// ============================================================================

/// Distancia, em turnos a partir de agora, de cada casa ate a origem mais
/// proxima. As origens comecam em `start_turn`. Casas sem caminho ficam
/// com `UNREACHABLE`.
///
/// A BFS anda em "ondas": primeiro todas as casas a 1 passo, depois a 2...
/// Uma casa so entra na onda `t` se ja estiver livre no turno `t`.
fn distances_from(grid: &Grid, origins: &[Coord], start_turn: u32, my_delay: u32) -> Vec<u32> {
    let mut distance = vec![UNREACHABLE; grid.cell_count()];
    let mut queue: VecDeque<Coord> = VecDeque::new();

    for origin in origins {
        if let Some(index) = grid.index_of(*origin) {
            distance[index] = start_turn;
            queue.push_back(*origin);
        }
    }

    while let Some(cell) = queue.pop_front() {
        let here = match grid.index_of(cell) {
            Some(index) => distance[index],
            None => continue,
        };
        for direction in ALL_DIRECTIONS {
            let next = direction.step(cell);
            let next_index = match grid.index_of(next) {
                Some(index) => index,
                None => continue,
            };
            if distance[next_index] != UNREACHABLE {
                continue;
            }
            if grid.can_enter(next, here + 1, my_delay) {
                distance[next_index] = here + 1;
                queue.push_back(next);
            }
        }
    }

    distance
}

/// A comida mais proxima que alcancamos ANTES das cobras maiores ou iguais.
/// Se nenhuma for "nossa", devolve a mais proxima de todas.
fn nearest_food(grid: &Grid, state: &GameState, mine: &[u32], enemy: &[u32]) -> Option<u32> {
    let mut best_ours: Option<u32> = None;
    let mut best_any: Option<u32> = None;

    for food in &state.board.food {
        let index = match grid.index_of(*food) {
            Some(index) => index,
            None => continue,
        };
        let ours = mine[index];
        if ours == UNREACHABLE {
            continue;
        }
        if best_any.map_or(true, |best| ours < best) {
            best_any = Some(ours);
        }
        // >>> Empate na distancia nao serve: chegariamos juntas e a menor (ou as
        // >>> duas, se iguais) morreria. So conta se chegamos estritamente antes.
        if ours < enemy[index] && best_ours.map_or(true, |best| ours < best) {
            best_ours = Some(ours);
        }
    }

    if best_ours.is_some() {
        best_ours
    } else {
        best_any
    }
}

/// `true` se a casa `target` e vizinha de alguma das cabecas da lista.
fn is_next_to_any(target: Coord, heads: &[Coord]) -> bool {
    for head in heads {
        let dx = (head.x - target.x).abs();
        let dy = (head.y - target.y).abs();
        if dx + dy == 1 {
            return true;
        }
    }
    false
}

/// Passos (sem contar obstaculos) de uma casa ate o centro do tabuleiro.
fn distance_to_center(grid: &Grid, c: Coord) -> i64 {
    let center_x = (grid.width - 1) / 2;
    let center_y = (grid.height - 1) / 2;
    ((c.x - center_x).abs() + (c.y - center_y).abs()) as i64
}

/// Texto curto para o `shout`: versao, direcao, espaco, comida e alertas.
fn describe(info: &MoveInfo) -> String {
    let food = match info.food_distance {
        Some(distance) => distance.to_string(),
        None => "-".to_string(),
    };
    let mut text = format!(
        "{} {} esp{} com{}",
        VERSION_TAG,
        info.direction.as_str(),
        info.space,
        food
    );
    if info.trapped {
        text.push_str(" beco");
    }
    if info.head_risk {
        text.push_str(" risco");
    }
    text
}

// ============================================================================
// ||  BLOCO: JOGADA DE EMERGENCIA
// ||  O QUE FAZ: escolhe uma direcao valida com o minimo de calculo.
// ||  POR QUE:   se algo der muito errado (panico, tabuleiro estranho),
// ||             ainda respondemos algo que nao sai do tabuleiro.
// ============================================================================

pub fn emergency_move(state: &GameState) -> Direction {
    let head = state.you.head;
    let neck = state.you.body.get(1).copied();
    for direction in ALL_DIRECTIONS {
        let target = direction.step(head);
        let inside = target.x >= 0
            && target.y >= 0
            && target.x < state.board.width
            && target.y < state.board.height;
        if inside && Some(target) != neck {
            return direction;
        }
    }
    Direction::Up
}

// ============================================================================
// ||  BLOCO: TESTES DE CENARIO
// ||  O QUE FAZ: tabuleiros pequenos desenhados no comentario, com a jogada
// ||             esperada ou proibida.
// ||  POR QUE:   cada comportamento importante fica garantido para sempre.
// ||  LEGENDA:   H = nossa cabeca, n = pescoco, b = corpo, t = cauda,
// ||             E = cabeca adversaria, e = corpo adversario, F = comida.
// ||             O y cresce para CIMA: a linha de baixo do desenho e y=0.
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Battlesnake, Board, Game};
    use std::collections::HashMap;

    /// Cria uma cobra a partir das casas do corpo, da cabeca ate a cauda.
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
        }
    }

    /// Monta um tabuleiro 11x11 com a nossa cobra, as adversarias e a comida.
    fn state(me: Battlesnake, others: Vec<Battlesnake>, food: &[(i32, i32)]) -> GameState {
        let mut snakes = vec![me.clone()];
        snakes.extend(others);
        GameState {
            game: Game {
                id: "teste".to_string(),
                ruleset: HashMap::new(),
                map: None,
                timeout: 500,
            },
            turn: 10,
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

    /// A analise de uma direcao especifica.
    fn info_for(state: &GameState, direction: Direction) -> MoveInfo {
        let infos = analyze_moves(state);
        infos.into_iter().find(|info| info.direction == direction).unwrap()
    }

    fn chosen(state: &GameState) -> Direction {
        choose_move(state).direction
    }

    #[test]
    fn nao_sai_do_tabuleiro_no_canto() {
        // y=1  . . .
        // y=0  H n t
        let me = snake("eu", &[(0, 0), (1, 0), (2, 0)], 100);
        let s = state(me, vec![], &[]);
        assert!(!info_for(&s, Direction::Left).legal);
        assert!(!info_for(&s, Direction::Down).legal);
        assert_eq!(chosen(&s), Direction::Up);
    }

    #[test]
    fn nao_bate_no_proprio_corpo() {
        // y=3  . t b b
        // y=2  . . H b
        // y=1  . . n b
        //      0 1 2 3
        let me = snake("eu", &[(2, 2), (2, 1), (3, 1), (3, 2), (3, 3), (2, 3), (1, 3)], 100);
        let s = state(me, vec![], &[]);
        assert!(!info_for(&s, Direction::Up).legal);
        assert!(!info_for(&s, Direction::Right).legal);
        assert_eq!(chosen(&s), Direction::Left);
    }

    #[test]
    fn nao_bate_em_outra_cobra() {
        // y=6  . . . E e e e e
        // y=5  . . . . . H . .
        // y=4  . . . . . n . .
        // y=3  . . . . . t . .
        //      0 1 2 3 4 5 6 7
        let me = snake("eu", &[(5, 5), (5, 4), (5, 3)], 100);
        let enemy = snake("ela", &[(3, 6), (4, 6), (5, 6), (6, 6), (7, 6)], 100);
        let s = state(me, vec![enemy], &[]);
        assert!(!info_for(&s, Direction::Up).legal);
        assert_ne!(chosen(&s), Direction::Up);
    }

    #[test]
    fn pode_entrar_na_casa_da_propria_cauda() {
        // y=1  n b
        // y=0  H t     <- a cauda sai do lugar neste turno
        let me = snake("eu", &[(0, 0), (0, 1), (1, 1), (1, 0)], 100);
        let s = state(me, vec![], &[]);
        assert!(info_for(&s, Direction::Right).legal);
        assert_eq!(chosen(&s), Direction::Right);
    }

    #[test]
    fn nao_entra_na_cauda_que_acabou_de_comer() {
        // Mesmo desenho, mas a cobra comeu no turno passado: a cauda esta
        // empilhada (2 pedacos em (1,0)) e NAO sai do lugar neste turno.
        let me = snake("eu", &[(0, 0), (0, 1), (1, 1), (1, 0), (1, 0)], 100);
        let s = state(me, vec![], &[]);
        assert!(!info_for(&s, Direction::Right).legal);
    }

    #[test]
    fn foge_de_cabeca_com_cabeca_contra_cobra_maior() {
        // y=8  . e      a adversaria (tamanho 4) pode descer para (5,6),
        // y=7  . E      a mesma casa onde o nosso "up" colocaria a cabeca
        // y=6  . .
        // y=5  . H
        // y=4  . n
        // y=3  . t
        //      4 5
        let me = snake("eu", &[(5, 5), (5, 4), (5, 3)], 100);
        let enemy = snake("ela", &[(5, 7), (5, 8), (5, 9), (5, 10)], 100);
        let s = state(me, vec![enemy], &[]);
        assert!(info_for(&s, Direction::Up).head_risk);
        assert_ne!(chosen(&s), Direction::Up);
    }

    #[test]
    fn foge_de_cabeca_com_cabeca_contra_cobra_do_mesmo_tamanho() {
        // Mesmo tamanho: se as cabecas se encontram, as DUAS morrem.
        let me = snake("eu", &[(5, 5), (5, 4), (5, 3)], 100);
        let enemy = snake("ela", &[(5, 7), (5, 8), (5, 9)], 100);
        let s = state(me, vec![enemy], &[]);
        assert!(info_for(&s, Direction::Up).head_risk);
        assert_ne!(chosen(&s), Direction::Up);
    }

    #[test]
    fn ataca_cobra_menor_quando_e_seguro() {
        // A adversaria tem tamanho 2. Se ela descer para (5,6) e nos subirmos,
        // ela morre e nos sobrevivemos.
        let me = snake("eu", &[(5, 5), (5, 4), (5, 3)], 100);
        let enemy = snake("ela", &[(5, 7), (5, 8)], 100);
        let s = state(me, vec![enemy], &[]);
        assert!(info_for(&s, Direction::Up).kill_chance);
        assert_eq!(chosen(&s), Direction::Up);
    }

    #[test]
    fn nao_entra_em_beco() {
        // y=6  . . e . . . . . . .
        // y=5  . . e . . . . . . .
        // y=4  . . e . . . . . . .
        // y=3  . . e . . . . . . .
        // y=2  E e e . . . . . . .     <- parede formada pela adversaria
        // y=1  . . . H . . . . . .
        // y=0  . . . n b b b b b t
        //      0 1 2 3 4 5 6 7 8 9
        // Ir para a esquerda entra num bolsao de 6 casas, menor que o nosso
        // comprimento (8): beco sem saida.
        let me = snake(
            "eu",
            &[(3, 1), (3, 0), (4, 0), (5, 0), (6, 0), (7, 0), (8, 0), (9, 0)],
            100,
        );
        let enemy = snake("ela", &[(0, 2), (1, 2), (2, 2), (2, 3), (2, 4), (2, 5), (2, 6)], 100);
        let s = state(me, vec![enemy], &[]);
        let left = info_for(&s, Direction::Left);
        assert!(left.legal);
        assert!(left.trapped, "o bolsao tem {} casas", left.space);
        assert_ne!(chosen(&s), Direction::Left);
    }

    #[test]
    fn flood_fill_sabe_que_a_cauda_anda() {
        // y=3  e e e
        // y=2  H . e     <- so da para ir para (1,2). Contando as casas livres
        // y=1  n t e        AGORA, o espaco seria 1 (beco!). Mas a nossa cauda
        // y=0  b b E        anda e vai abrindo caminho: o espaco real e maior.
        //      0 1 2
        let me = snake("eu", &[(0, 2), (0, 1), (0, 0), (1, 0), (1, 1)], 100);
        let enemy = snake(
            "ela",
            &[(2, 0), (2, 1), (2, 2), (2, 3), (1, 3), (0, 3), (0, 4), (0, 5), (0, 6), (0, 7)],
            100,
        );
        let s = state(me, vec![enemy], &[]);
        let right = info_for(&s, Direction::Right);
        assert!(right.legal);
        assert!(right.space >= 5, "espaco calculado: {}", right.space);
        assert!(!right.trapped);
        assert_eq!(chosen(&s), Direction::Right);
    }

    #[test]
    fn vai_atras_da_comida_com_fome() {
        // y=8  . F .
        // y=7  . . .
        // y=6  . . .
        // y=5  . H .     vida 10: precisa comer
        // y=4  . n .
        //      4 5 6
        let me = snake("eu", &[(5, 5), (5, 4), (5, 3)], 10);
        let s = state(me, vec![], &[(5, 8)]);
        assert_eq!(chosen(&s), Direction::Up);
    }

    #[test]
    fn prefere_comida_que_alcanca_primeiro() {
        // A comida F1 (5,5) esta a 3 passos de nos e a 3 da adversaria maior:
        // chegariamos juntas e perderiamos. A F2 (2,9) e so nossa.
        //
        // y=9  . . F2 . . . . . .
        // y=5  . . H  . . F1 . . E
        // y=4  . . n  . . .  . . e
        // y=3  . . t  . . .  . . e
        //      0 1 2  3 4 5  6 7 8
        let me = snake("eu", &[(2, 5), (2, 4), (2, 3)], 10);
        let enemy = snake("ela", &[(8, 5), (8, 4), (8, 3), (8, 2)], 100);
        let s = state(me, vec![enemy], &[(5, 5), (2, 9)]);
        assert_eq!(chosen(&s), Direction::Up);
    }
}
