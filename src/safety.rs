// ============================================================================
// ||  BLOCO: PESOS DA DECISAO
// ||  O QUE FAZ: todos os numeros que definem o "jeito" da cobra, juntos.
// ||  POR QUE:   para ajustar a cobra sem cacar numeros no codigo, e para
// ||             comparar versoes no self-play (a de ontem contra a de hoje).
// ============================================================================

/// Os pesos de uma versao da cobra.
#[derive(Debug, Clone, Copy)]
pub struct Settings {
    /// Etiqueta que vai no `shout`, para saber qual versao jogou.
    pub tag: &'static str,
    /// Abaixo desta vida a cobra procura comida.
    pub hungry_health: i32,
    /// Abaixo desta vida comer vale mais que espaco (nunca mais que seguranca).
    pub critical_health: i32,
    /// Queremos ser pelo menos esta quantidade maior que a maior adversaria.
    pub wanted_length_lead: i32,
    /// Espaco "confortavel" = comprimento * multiplier + extra.
    pub comfort_multiplier: i32,
    pub comfort_extra: i32,
    /// Quanto vale cada passo mais perto da comida (com fome / vida critica).
    pub food_weight: i64,
    pub food_weight_critical: i64,
    /// Bonus por mirar a casa onde uma cobra MENOR pode colocar a cabeca.
    pub kill_bonus: i64,
    /// Desempate: cada passo mais perto do centro.
    pub center_weight: i64,
    /// Flood fill cauteloso: as casas vizinhas das cabecas adversarias
    /// contam como ocupadas (a cabeca delas tambem anda).
    pub cautious_flood_fill: bool,
    /// Territorio (Voronoi): cada casa que alcancamos antes das adversarias.
    pub territory_weight: i64,
    /// Caca: quando somos a maior, cada passo mais perto da cabeca adversaria.
    /// Multiplicado pela urgencia, que cresce ate o turno 99.
    pub hunt_weight: i64,
    /// Bonus por deixar uma adversaria presa num espaco menor que ela.
    pub trap_enemy_bonus: i64,
}

/// A versao que joga agora.
pub const SETTINGS: Settings = Settings {
    tag: "v0606",
    hungry_health: 50,
    critical_health: 25,
    wanted_length_lead: 3,
    comfort_multiplier: 2,
    comfort_extra: 4,
    food_weight: 300,
    food_weight_critical: 1_000,
    kill_bonus: 300,
    center_weight: 1,
    cautious_flood_fill: true,
    territory_weight: 10,
    hunt_weight: 20,
    trap_enemy_bonus: 50_000,
};

/// A versao que jogou na arena em 05/10, guardada para comparar no self-play.
#[cfg(test)]
pub const SETTINGS_V0510: Settings = Settings {
    tag: "v0510",
    hungry_health: 50,
    critical_health: 25,
    wanted_length_lead: 2,
    comfort_multiplier: 2,
    comfort_extra: 4,
    food_weight: 3,
    food_weight_critical: 1_000,
    kill_bonus: 60,
    center_weight: 1,
    cautious_flood_fill: false,
    territory_weight: 0,
    hunt_weight: 0,
    trap_enemy_bonus: 0,
};

/// Notas das camadas de seguranca. Cada camada vale mais que tudo abaixo dela.
const SCORE_NOT_TRAPPED: i64 = 1_000_000;
const SCORE_NO_HEAD_RISK: i64 = 100_000;
const SCORE_PER_SPACE_CELL: i64 = 100;
/// Comida: a nota cresce quanto mais perto, ate este horizonte de passos.
const FOOD_HORIZON: i64 = 30;
/// A urgencia da caca sobe 1 ponto a cada tantos turnos (1 no inicio, 4 no turno 99).
const TURNS_PER_URGENCY_STEP: i32 = 33;

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
    /// Casas que alcancamos antes das adversarias, menos as que elas alcancam antes.
    pub territory: i32,
    /// Quantas adversarias ficam presas (espaco menor que o corpo) depois desta jogada.
    pub enemies_trapped: i32,
    /// Somos a maior: passos ate a cabeca adversaria mais proxima.
    pub hunt_distance: Option<i32>,
    pub score: i64,
}

/// A decisao final: a direcao e o texto curto para o `shout`.
pub struct Decision {
    pub direction: Direction,
    pub shout: String,
}

/// Uma adversaria, resumida: onde esta a cabeca e qual o tamanho.
struct Enemy {
    head: Coord,
    length: i32,
}

// ============================================================================
// ||  BLOCO: ESCOLHA DA JOGADA
// ||  O QUE FAZ: analisa as 4 direcoes e escolhe a de maior nota.
// ||  POR QUE:   nunca escolhe uma jogada que mata agora se existir outra,
// ||             foge de becos e de cobras maiores, come quando precisa e,
// ||             sendo a maior, cerca e caca a adversaria.
// ============================================================================

/// A jogada da versao atual.
pub fn choose_move(state: &GameState) -> Decision {
    choose_move_with(state, &SETTINGS)
}

/// A jogada com os pesos de uma versao qualquer (o self-play usa isto).
pub fn choose_move_with(state: &GameState, settings: &Settings) -> Decision {
    let infos = analyze_moves_with(state, settings);

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
            shout: describe(info, settings),
        },
        // >>> Nenhuma direcao legal: qualquer jogada morre. Respondemos algo valido.
        None => Decision {
            direction: emergency_move(state),
            shout: format!("{} sem saida", settings.tag),
        },
    }
}

/// Analisa as 4 direcoes com os pesos da versao atual.
#[cfg(test)]
pub fn analyze_moves(state: &GameState) -> Vec<MoveInfo> {
    analyze_moves_with(state, &SETTINGS)
}

/// Analisa as 4 direcoes. Devolve uma lista vazia se o tabuleiro for invalido.
pub fn analyze_moves_with(state: &GameState, settings: &Settings) -> Vec<MoveInfo> {
    let mut infos = Vec::new();
    let grid = match Grid::from_state(state) {
        Some(grid) => grid,
        None => return infos,
    };

    let me = &state.you;
    let my_head = me.head;
    let my_length = me.body.len() as i32;

    // Separa as adversarias: as maiores ou iguais sao perigo; as menores, alvo.
    let mut enemies: Vec<Enemy> = Vec::new();
    let mut big_heads: Vec<Coord> = Vec::new();
    let mut small_heads: Vec<Coord> = Vec::new();
    let mut all_heads: Vec<Coord> = Vec::new();
    let mut longest_enemy = 0;
    for snake in &state.board.snakes {
        if snake.id == me.id {
            continue;
        }
        let length = snake.body.len() as i32;
        longest_enemy = longest_enemy.max(length);
        if length >= my_length {
            big_heads.push(snake.head);
        } else {
            small_heads.push(snake.head);
        }
        all_heads.push(snake.head);
        enemies.push(Enemy { head: snake.head, length });
    }

    // >>> Grade cautelosa: as casas onde cada cabeca adversaria PODE entrar no
    // >>> proximo turno contam como ocupadas pelo tempo que o corpo dela levaria
    // >>> para sair de la.
    let mut cautious = grid.clone();
    if settings.cautious_flood_fill {
        for enemy in &enemies {
            for direction in ALL_DIRECTIONS {
                let cell = direction.step(enemy.head);
                if grid.can_enter(cell, 1, 0) {
                    cautious.occupy_until(cell, enemy.length as u32 + 1);
                }
            }
        }
    }

    // >>> Distancia de cada casa ate a cabeca maior-ou-igual mais proxima.
    // >>> Serve para saber quais comidas conseguimos pegar ANTES delas.
    let enemy_distance = distances_from(&grid, &big_heads, 0, 0);

    let wants_food =
        me.health < settings.hungry_health || my_length < longest_enemy + settings.wanted_length_lead;
    let critical = me.health < settings.critical_health;
    let i_am_biggest = !enemies.is_empty() && my_length > longest_enemy;
    let urgency = 1 + (state.turn.max(0) / TURNS_PER_URGENCY_STEP) as i64;

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
            territory: 0,
            enemies_trapped: 0,
            hunt_distance: None,
            score: 0,
        };

        if info.legal {
            // >>> Se esta jogada come, a nossa cauda fica parada 1 turno:
            // >>> todas as casas do nosso corpo demoram 1 turno a mais para liberar.
            let my_delay = if info.eats { 1 } else { 0 };
            let length_after = my_length + if info.eats { 1 } else { 0 };
            let my_distance = distances_from(&cautious, &[target], 1, my_delay);

            info.space = count_reachable(&my_distance);
            info.trapped = info.space < length_after;
            info.food_distance = nearest_food(&grid, state, &my_distance, &enemy_distance);

            if !enemies.is_empty() && (settings.territory_weight != 0 || settings.trap_enemy_bonus != 0) {
                // "E se a nossa cabeca estiver aqui?": a casa vira corpo nosso.
                let mut after = grid.clone();
                after.occupy_until(target, length_after as u32 + 1);

                let their_distance = distances_from(&after, &all_heads, 0, 0);
                info.territory = territory(&my_distance, &their_distance);

                for enemy in &enemies {
                    let reach = distances_from(&after, &[enemy.head], 0, 0);
                    // >>> -1 porque a contagem inclui a casa da propria cabeca dela.
                    if count_reachable(&reach) - 1 < enemy.length {
                        info.enemies_trapped += 1;
                    }
                }
            }

            if i_am_biggest {
                let mut nearest = i32::MAX;
                for enemy in &enemies {
                    nearest = nearest.min(manhattan(target, enemy.head));
                }
                info.hunt_distance = Some(nearest);
            }

            info.score = score_move(&info, &grid, my_length, wants_food, critical, urgency, settings);
        }

        infos.push(info);
    }

    infos
}

/// A nota de uma jogada legal, em camadas: cada camada vale mais que
/// tudo o que vem depois dela.
fn score_move(
    info: &MoveInfo,
    grid: &Grid,
    my_length: i32,
    wants_food: bool,
    critical: bool,
    urgency: i64,
    settings: &Settings,
) -> i64 {
    let mut score = 0;

    // Camada 1: nao entrar em beco.
    if !info.trapped {
        score += SCORE_NOT_TRAPPED;
    }
    // Camada 2: nao arriscar cabeca com cabeca contra cobra maior ou igual.
    if !info.head_risk {
        score += SCORE_NO_HEAD_RISK;
    }
    // Camada 3: deixar adversarias presas (vitoria quase certa, sem arriscar a nossa).
    score += info.enemies_trapped as i64 * settings.trap_enemy_bonus;

    // Camada 4: espaco, ate o limite do confortavel.
    let comfortable = my_length * settings.comfort_multiplier + settings.comfort_extra;
    score += info.space.min(comfortable) as i64 * SCORE_PER_SPACE_CELL;

    // Camada 5: comida, territorio e caca disputam entre si.
    if wants_food || critical {
        if let Some(distance) = info.food_distance {
            let closeness = (FOOD_HORIZON - distance as i64).max(0);
            let weight = if critical { settings.food_weight_critical } else { settings.food_weight };
            score += closeness * weight;
        }
    }
    score += info.territory as i64 * settings.territory_weight;
    if let Some(distance) = info.hunt_distance {
        score -= distance as i64 * settings.hunt_weight * urgency;
    }

    // Desempates: atacar cobra menor e ficar perto do centro.
    if info.kill_chance && !info.head_risk {
        score += settings.kill_bonus;
    }
    score -= distance_to_center(grid, info.target) * settings.center_weight;

    score
}

// ============================================================================
// ||  BLOCO: BUSCA EM LARGURA (BFS) QUE SABE QUE AS CAUDAS ANDAM
// ||  O QUE FAZ: calcula em quantos turnos chegamos a cada casa.
// ||  POR QUE:   com isso medimos o espaco (flood fill), a distancia ate a
// ||             comida e o territorio de cada cobra.
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

/// Quantas casas a BFS alcancou.
fn count_reachable(distance: &[u32]) -> i32 {
    distance.iter().filter(|d| **d != UNREACHABLE).count() as i32
}

/// Territorio (Voronoi): casas onde chegamos antes, menos casas onde elas
/// chegam antes. Empate nao conta para ninguem.
fn territory(mine: &[u32], theirs: &[u32]) -> i32 {
    let mut result = 0;
    for (m, t) in mine.iter().zip(theirs.iter()) {
        if m < t {
            result += 1;
        } else if t < m {
            result -= 1;
        }
    }
    result
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
    heads.iter().any(|head| manhattan(*head, target) == 1)
}

/// Passos entre duas casas, sem contar obstaculos.
fn manhattan(a: Coord, b: Coord) -> i32 {
    (a.x - b.x).abs() + (a.y - b.y).abs()
}

/// Passos (sem contar obstaculos) de uma casa ate o centro do tabuleiro.
fn distance_to_center(grid: &Grid, c: Coord) -> i64 {
    let center = Coord { x: (grid.width - 1) / 2, y: (grid.height - 1) / 2 };
    manhattan(c, center) as i64
}

/// Texto curto para o `shout`: versao, direcao, espaco, territorio, comida e alertas.
fn describe(info: &MoveInfo, settings: &Settings) -> String {
    let food = match info.food_distance {
        Some(distance) => distance.to_string(),
        None => "-".to_string(),
    };
    let mut text = format!(
        "{} {} esp{} ter{} com{}",
        settings.tag,
        info.direction.as_str(),
        info.space,
        info.territory,
        food
    );
    if info.trapped {
        text.push_str(" beco");
    }
    if info.head_risk {
        text.push_str(" risco");
    }
    if info.enemies_trapped > 0 {
        text.push_str(" cerco");
    }
    if info.hunt_distance.is_some() {
        text.push_str(" caca");
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

    #[test]
    fn nao_entra_no_corredor_da_partida_contra_o_ian() {
        // Tabuleiro REAL do turno 66 da partida c6dd4921 (05/10), contra Ian Augusto.
        // A v0510 desceu para (2,7) e ficou presa no canto por 2 turnos.
        // A saida boa era a direita: a cauda do Ian (4,8) e a nossa (5,7)
        // liberam caminho para o tabuleiro aberto.
        //
        // y=10  . . b b . b
        // y= 9  . . b b b b
        // y= 8  . . H . e b
        // y= 7  . . . e e t
        // y= 6  . e e e . .
        // y= 5  . E . . . .
        // y= 4  . F . . . . . F
        //       0 1 2 3 4 5 6 7
        let me = snake(
            "eu",
            &[(2, 8), (2, 9), (2, 10), (3, 10), (3, 9), (4, 9), (4, 10), (5, 10), (5, 9), (5, 8), (5, 7)],
            98,
        );
        let ian = snake("ian", &[(1, 5), (1, 6), (2, 6), (3, 6), (3, 7), (4, 7), (4, 8)], 74);
        let mut s = state(me, vec![ian], &[(7, 4), (1, 4), (8, 0), (8, 5)]);
        s.turn = 66;

        // A versao antiga achava que descer era seguro...
        let old = analyze_moves_with(&s, &SETTINGS_V0510);
        let old_down = old.iter().find(|i| i.direction == Direction::Down).unwrap();
        assert!(!old_down.trapped);

        // ...a nova percebe o beco e vai para a direita.
        assert!(info_for(&s, Direction::Down).trapped);
        assert_eq!(chosen(&s), Direction::Right);
    }

    #[test]
    fn prende_a_adversaria_contra_a_parede() {
        // A adversaria (tamanho 5) anda para a esquerda no corredor entre a
        // parede de baixo e o nosso corpo. A unica saida do corredor e (0,1).
        // Indo para a esquerda, a nossa cabeca fecha a saida: ela fica com
        // 3 casas livres, menos que o tamanho dela.
        //
        // y=2  . . . . . . . .
        // y=1  . H b b b b b t
        // y=0  . . . E e e e e
        //      0 1 2 3 4 5 6 7
        let me = snake("eu", &[(1, 1), (2, 1), (3, 1), (4, 1), (5, 1), (6, 1), (7, 1)], 100);
        let enemy = snake("ela", &[(3, 0), (4, 0), (5, 0), (6, 0), (7, 0)], 100);
        let s = state(me, vec![enemy], &[]);
        let left = info_for(&s, Direction::Left);
        assert_eq!(left.enemies_trapped, 1, "{:?}", left);
        assert!(!left.trapped);
        // Descer para (1,0) tambem fecha o corredor (ainda mais perto dela).
        // Qualquer uma das duas serve; subir deixaria a saida aberta.
        let choice = chosen(&s);
        assert!(info_for(&s, choice).enemies_trapped >= 1, "escolheu {:?}", choice);
        assert_ne!(choice, Direction::Up);
    }

    #[test]
    fn caca_a_adversaria_menor() {
        // Somos bem maiores (7 contra 3) e estamos de vida cheia: sem fome,
        // a cobra vai na direcao da adversaria em vez de ficar passeando.
        let me = snake("eu", &[(5, 5), (5, 4), (5, 3), (5, 2), (5, 1), (4, 1), (3, 1)], 100);
        let enemy = snake("ela", &[(9, 5), (9, 6), (9, 7)], 100);
        let mut s = state(me, vec![enemy], &[]);
        s.turn = 80;
        assert_eq!(chosen(&s), Direction::Right);
    }
}
