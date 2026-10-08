// ============================================================================
// ||  BLOCO: GEOMETRIA DO TABULEIRO
// ||  O QUE FAZ: direcoes, passos e a grade com "quando cada casa fica livre".
// ||  POR QUE:   o resto da cobra pensa em casas e passos; aqui fica a base
// ||             que todo mundo usa, com as regras do jogo sobre caudas.
// ============================================================================

use crate::models::{Battlesnake, Coord, GameState};

/// Distancia de uma casa que nao da para alcancar.
pub const UNREACHABLE: u32 = u32::MAX;

/// Maior tabuleiro que aceitamos (em numero de casas). O padrao e 11x11 = 121.
/// Acima disso a requisicao e estranha e usamos so a jogada de emergencia.
pub const MAX_CELLS: i32 = 2500;

/// As quatro direcoes possiveis. Um `enum` e um tipo que so pode assumir
/// um dos valores listados: aqui, exatamente uma das 4 direcoes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Up,
    Down,
    Left,
    Right,
}

/// As 4 direcoes numa lista, para percorrer com `for`.
pub const ALL_DIRECTIONS: [Direction; 4] = [
    Direction::Up,
    Direction::Down,
    Direction::Left,
    Direction::Right,
];

impl Direction {
    /// O texto que a arena espera na resposta do /move.
    pub fn as_str(self) -> &'static str {
        match self {
            Direction::Up => "up",
            Direction::Down => "down",
            Direction::Left => "left",
            Direction::Right => "right",
        }
    }

    /// A casa vizinha de `from` nesta direcao.
    // >>> A origem (0,0) fica no canto INFERIOR esquerdo: "up" aumenta o y.
    pub fn step(self, from: Coord) -> Coord {
        match self {
            Direction::Up => Coord { x: from.x, y: from.y + 1 },
            Direction::Down => Coord { x: from.x, y: from.y - 1 },
            Direction::Left => Coord { x: from.x - 1, y: from.y },
            Direction::Right => Coord { x: from.x + 1, y: from.y },
        }
    }
}

// ============================================================================
// ||  BLOCO: GRADE COM O TEMPO DE LIBERACAO DE CADA CASA
// ||  O QUE FAZ: para cada casa, guarda daqui a quantos turnos ela fica livre.
// ||  POR QUE:   o corpo das cobras anda. Uma casa ocupada agora pode estar
// ||             livre quando a nossa cabeca chegar la. Sem isso, a cobra
// ||             acharia que esta presa quando so precisa seguir a cauda.
// ============================================================================

/// A grade do tabuleiro, guardada como um vetor linear:
/// a casa (x, y) fica na posicao `y * width + x`.
/// `Clone` permite fazer uma copia da grade para marcar "e se" sem estragar a original.
#[derive(Clone)]
pub struct Grid {
    pub width: i32,
    pub height: i32,
    /// Daqui a quantos turnos a casa fica livre. 0 = livre agora.
    free_at: Vec<u32>,
    /// `true` se a casa esta ocupada pelo NOSSO corpo.
    mine: Vec<bool>,
}

impl Grid {
    /// Monta a grade a partir do estado que a arena mandou.
    /// Devolve `None` se o tabuleiro for vazio ou grande demais.
    pub fn from_state(state: &GameState) -> Option<Grid> {
        let width = state.board.width;
        let height = state.board.height;
        if width <= 0 || height <= 0 || width * height > MAX_CELLS {
            return None;
        }

        let cells = (width * height) as usize;
        let mut grid = Grid {
            width,
            height,
            free_at: vec![0; cells],
            mine: vec![false; cells],
        };

        let mut me_is_listed = false;
        for snake in &state.board.snakes {
            let is_me = snake.id == state.you.id;
            if is_me {
                me_is_listed = true;
            }
            grid.add_body(&snake.body, is_me);
        }
        // >>> Normalmente a nossa cobra tambem vem em `board.snakes`. Se nao vier,
        // >>> colocamos o corpo dela na grade mesmo assim.
        if !me_is_listed {
            grid.add_body(&state.you.body, true);
        }

        Some(grid)
    }

    /// Marca um corpo na grade.
    ///
    /// Regra do jogo: a cada turno a cauda anda uma casa. Num corpo de
    /// tamanho `len`, o pedaco de indice `i` (0 = cabeca) sai do tabuleiro
    /// daqui a `len - i` turnos.
    // >>> Se a cobra acabou de comer, os dois ultimos pedacos estao na mesma
    // >>> casa. Pegando o MAIOR tempo entre os pedacos de uma casa, a cauda
    // >>> empilhada demora um turno a mais para sair, como manda a regra.
    fn add_body(&mut self, body: &[Coord], is_me: bool) {
        let len = body.len() as u32;
        for (i, segment) in body.iter().enumerate() {
            if let Some(index) = self.index_of(*segment) {
                let leaves_in = len - i as u32;
                if leaves_in > self.free_at[index] {
                    self.free_at[index] = leaves_in;
                }
                if is_me {
                    self.mine[index] = true;
                }
            }
        }
    }

    /// Marca uma casa como ocupada ate daqui a `turns` turnos (se ja estiver
    /// ocupada por mais tempo, fica como esta). Usado para os "e se":
    /// "e se a nossa cabeca for para ca?", "e se a adversaria vier para ca?".
    pub fn occupy_until(&mut self, c: Coord, turns: u32) {
        if let Some(index) = self.index_of(c) {
            if turns > self.free_at[index] {
                self.free_at[index] = turns;
            }
        }
    }

    /// Numero total de casas do tabuleiro.
    pub fn cell_count(&self) -> usize {
        self.free_at.len()
    }

    /// A posicao da casa no vetor linear, ou `None` se estiver fora do tabuleiro.
    /// `Option` e o jeito do Rust dizer "pode ter um valor ou nao ter nada".
    pub fn index_of(&self, c: Coord) -> Option<usize> {
        if c.x < 0 || c.y < 0 || c.x >= self.width || c.y >= self.height {
            return None;
        }
        Some((c.y * self.width + c.x) as usize)
    }

    /// A nossa cabeca pode entrar nesta casa daqui a `turns` turnos?
    ///
    /// `my_delay` atrasa so as casas do nosso corpo: se comermos agora,
    /// a nossa cauda fica parada um turno, e tudo o que e nosso demora 1 a mais.
    pub fn can_enter(&self, c: Coord, turns: u32, my_delay: u32) -> bool {
        match self.index_of(c) {
            Some(index) => self.can_enter_index(index, turns, my_delay),
            None => false,
        }
    }

    /// O mesmo que `can_enter`, recebendo o indice da casa (ja dentro do tabuleiro).
    pub fn can_enter_index(&self, index: usize, turns: u32, my_delay: u32) -> bool {
        let mut ready_at = self.free_at[index];
        if self.mine[index] {
            ready_at = ready_at.saturating_add(my_delay);
        }
        ready_at <= turns
    }
}

// ============================================================================
// ||  BLOCO: BUSCA EM LARGURA (BFS) QUE SABE QUE AS CAUDAS ANDAM
// ||  O QUE FAZ: calcula em quantos turnos uma cabeca chega a cada casa.
// ||  POR QUE:   com isso medimos espaco (flood fill), distancia ate a comida
// ||             e o territorio de cada cobra. Usada pela rede de seguranca
// ||             e pela avaliacao da busca.
// ============================================================================

/// Distancia, em turnos a partir de agora, de cada casa ate a origem mais
/// proxima. As origens comecam em `start_turn`. Casas sem caminho ficam
/// com `UNREACHABLE`.
///
/// A BFS anda em "ondas": primeiro todas as casas a 1 passo, depois a 2...
/// Uma casa so entra na onda `t` se ja estiver livre no turno `t`.
pub fn distances_from(grid: &Grid, origins: &[Coord], start_turn: u32, my_delay: u32) -> Vec<u32> {
    let cells = grid.cell_count();
    let width = grid.width as usize;
    let mut distance = vec![UNREACHABLE; cells];
    // >>> A fila guarda o INDICE de cada casa (y * largura + x), nao a coordenada:
    // >>> assim os vizinhos saem de uma soma, sem refazer contas. Cada casa entra
    // >>> na fila no maximo uma vez, entao um vetor comum com um "ponteiro de
    // >>> leitura" (`next_to_read`) basta como fila.
    let mut queue: Vec<usize> = Vec::with_capacity(cells);

    for origin in origins {
        if let Some(index) = grid.index_of(*origin) {
            if distance[index] == UNREACHABLE {
                distance[index] = start_turn;
                queue.push(index);
            }
        }
    }

    let mut next_to_read = 0;
    while next_to_read < queue.len() {
        let index = queue[next_to_read];
        next_to_read += 1;
        let arrive = distance[index] + 1;
        let x = index % width;

        // >>> Os 4 vizinhos: acima (+largura), abaixo (-largura), esquerda (-1) e
        // >>> direita (+1). As condicoes evitam sair do tabuleiro e "dar a volta"
        // >>> de uma ponta da linha para a outra.
        let mut neighbors = [usize::MAX; 4];
        if index + width < cells {
            neighbors[0] = index + width;
        }
        if index >= width {
            neighbors[1] = index - width;
        }
        if x > 0 {
            neighbors[2] = index - 1;
        }
        if x + 1 < width {
            neighbors[3] = index + 1;
        }

        for next in neighbors {
            if next == usize::MAX || distance[next] != UNREACHABLE {
                continue;
            }
            if grid.can_enter_index(next, arrive, my_delay) {
                distance[next] = arrive;
                queue.push(next);
            }
        }
    }

    distance
}

/// Quantas casas a BFS alcancou.
pub fn count_reachable(distance: &[u32]) -> i32 {
    distance.iter().filter(|d| **d != UNREACHABLE).count() as i32
}

/// Territorio (Voronoi): casas onde chegamos antes, menos casas onde elas
/// chegam antes. Em empate de distancia, a casa e de quem for maior:
/// `tie_is_ours` diz se esse alguem somos nos.
pub fn territory(mine: &[u32], theirs: &[u32], tie_is_ours: bool) -> i32 {
    let mut result = 0;
    for (m, t) in mine.iter().zip(theirs.iter()) {
        if m < t || (m == t && *m != UNREACHABLE && tie_is_ours) {
            result += 1;
        } else if t < m {
            result -= 1;
        }
    }
    result
}

/// Passos entre duas casas, sem contar obstaculos.
pub fn manhattan(a: Coord, b: Coord) -> i32 {
    (a.x - b.x).abs() + (a.y - b.y).abs()
}

// ============================================================================
// ||  BLOCO: LIMPEZA DAS COBRAS JA ELIMINADAS
// ||  O QUE FAZ: tira da lista as cobras que com certeza ja morreram.
// ||  POR QUE:   a arena da Maua manda as cobras eliminadas junto com as vivas
// ||             (o Battlesnake oficial nao faz isso). Em 08/10 morremos por
// ||             tratar o "cadaver" de uma cobra como parede (partida 4e1ffb00).
// ||  CUIDADO:   tirar uma cobra VIVA seria pior (ignorariamos uma ameaca).
// ||             So sai quem tem sinal claro de morte.
// ============================================================================

/// Os nomes de campos extras (comparados sem maiusculas) que indicam morte
/// quando vem preenchidos. A arena usa estes nomes nos quadros das partidas.
const DEATH_FIELDS: [&str; 3] = ["eliminatedcause", "eliminated_cause", "death"];

/// Uma copia do estado sem as cobras eliminadas, e quantas foram tiradas.
pub fn remove_eliminated(state: &GameState) -> (GameState, usize) {
    let mut clean = state.clone();
    clean.board.snakes.retain(|snake| snake.id == state.you.id || !is_eliminated(state, snake));
    let removed = state.board.snakes.len() - clean.board.snakes.len();
    (clean, removed)
}

fn is_eliminated(state: &GameState, snake: &Battlesnake) -> bool {
    let head = match snake.body.first() {
        Some(head) => *head,
        None => return true,
    };
    // >>> Morreu de fome, ou bateu na parede (a cabeca ficou fora do tabuleiro).
    if snake.health <= 0 || head.x < 0 || head.y < 0 || head.x >= state.board.width || head.y >= state.board.height {
        return true;
    }
    // >>> Um campo extra da arena dizendo como ela morreu.
    for (key, value) in &snake.extra {
        if DEATH_FIELDS.contains(&key.to_lowercase().as_str()) {
            let filled = match value {
                serde_json::Value::Null => false,
                serde_json::Value::String(text) => !text.is_empty(),
                serde_json::Value::Bool(flag) => *flag,
                _ => true,
            };
            if filled {
                return true;
            }
        }
    }
    // >>> Duas cobras vivas nunca dividem uma casa, e nos estamos vivas: quem
    // >>> ocupa uma casa do nosso corpo so pode ser um cadaver.
    for segment in &snake.body {
        if state.you.body.contains(segment) {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Board, Game};
    use serde_json::json;
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

    fn state(snakes: Vec<Battlesnake>) -> GameState {
        GameState {
            game: Game { id: "limpeza".to_string(), ruleset: HashMap::new(), map: None, timeout: 500 },
            turn: 30,
            board: Board { width: 11, height: 11, food: vec![], hazards: vec![], snakes: snakes.clone() },
            you: snakes[0].clone(),
        }
    }

    fn remaining(s: &GameState) -> Vec<String> {
        remove_eliminated(s).0.board.snakes.iter().map(|x| x.id.clone()).collect()
    }

    #[test]
    fn tira_quem_bateu_na_parede_ou_morreu_de_fome() {
        let me = snake("eu", &[(6, 9), (6, 8), (6, 7)], 90);
        let wall = snake("parede", &[(5, 11), (5, 10), (5, 9)], 94);
        let hungry = snake("fome", &[(1, 1), (1, 2), (1, 3)], 0);
        let alive = snake("viva", &[(9, 1), (9, 2), (9, 3)], 50);
        assert_eq!(remaining(&state(vec![me, wall, hungry, alive])), vec!["eu", "viva"]);
    }

    #[test]
    fn tira_quem_divide_casa_com_o_nosso_corpo() {
        let me = snake("eu", &[(6, 9), (6, 8), (6, 7)], 90);
        let corpse = snake("cadaver", &[(5, 7), (6, 7), (7, 7)], 60);
        assert_eq!(remaining(&state(vec![me, corpse])), vec!["eu"]);
    }

    #[test]
    fn tira_quem_tem_campo_de_eliminacao_preenchido() {
        let me = snake("eu", &[(6, 9), (6, 8), (6, 7)], 90);
        let mut dead = snake("morta", &[(2, 2), (2, 3), (2, 4)], 60);
        dead.extra.insert("EliminatedCause".to_string(), json!("snake-collision"));
        let mut alive = snake("viva", &[(9, 1), (9, 2), (9, 3)], 50);
        alive.extra.insert("EliminatedCause".to_string(), json!(""));
        alive.extra.insert("Death".to_string(), json!(null));
        assert_eq!(remaining(&state(vec![me, dead, alive])), vec!["eu", "viva"]);
    }

    #[test]
    fn mantem_cobra_viva_passando_por_cima_de_cadaver() {
        // A viva esta com a cabeca em cima do corpo da morta: as duas parecem
        // "dividir casa", mas nao da para ter certeza de quem morreu.
        // Na duvida, ninguem sai (so sai a morta se tiver outro sinal).
        let me = snake("eu", &[(9, 9), (9, 8), (9, 7)], 90);
        let corpse = snake("cadaver", &[(5, 5), (5, 4), (5, 3)], 60);
        let alive = snake("viva", &[(5, 4), (4, 4), (3, 4)], 50);
        assert_eq!(remaining(&state(vec![me, corpse, alive])), vec!["eu", "cadaver", "viva"]);
    }
}
