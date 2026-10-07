// ============================================================================
// ||  BLOCO: GEOMETRIA DO TABULEIRO
// ||  O QUE FAZ: direcoes, passos e a grade com "quando cada casa fica livre".
// ||  POR QUE:   o resto da cobra pensa em casas e passos; aqui fica a base
// ||             que todo mundo usa, com as regras do jogo sobre caudas.
// ============================================================================

use crate::models::{Coord, GameState};
use std::collections::VecDeque;

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
        let index = match self.index_of(c) {
            Some(index) => index,
            None => return false,
        };
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
