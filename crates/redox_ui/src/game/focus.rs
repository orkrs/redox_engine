//! Keyboard / gamepad focus: moves a selection between buttons by direction.

use super::layout::Rect;

/// A navigation step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    /// Towards the top of the screen.
    Up,
    /// Towards the bottom.
    Down,
    /// Towards the left.
    Left,
    /// Towards the right.
    Right,
    /// Next button in tree order (Tab).
    Next,
    /// Previous button in tree order (Shift+Tab).
    Previous,
}

/// Which button is focused.
#[derive(Clone, Debug, Default)]
pub struct Focus {
    current: Option<String>,
}

impl Focus {
    /// The focused button id.
    pub fn current(&self) -> Option<&str> {
        self.current.as_deref()
    }

    /// Focuses a button directly (mouse hover, or a screen's default).
    pub fn set(&mut self, id: &str) {
        self.current = Some(id.into());
    }

    /// Keeps the focus valid: when the focused button is gone (a new screen), focus the first.
    pub fn ensure(&mut self, buttons: &[(String, Rect)]) {
        let valid = self
            .current
            .as_ref()
            .is_some_and(|id| buttons.iter().any(|(b, _)| b == id));
        if !valid {
            self.current = buttons.first().map(|(id, _)| id.clone());
        }
    }

    /// Moves the focus one step. Up/down/left/right pick the nearest button in that direction
    /// (off-axis distance counts double) and wrap to the far side at an edge.
    pub fn navigate(&mut self, direction: Direction, buttons: &[(String, Rect)]) {
        self.ensure(buttons);
        let Some(current) = self.current.clone() else {
            return;
        };
        let Some(index) = buttons.iter().position(|(id, _)| *id == current) else {
            return;
        };
        let target = match direction {
            Direction::Next => (index + 1) % buttons.len(),
            Direction::Previous => (index + buttons.len() - 1) % buttons.len(),
            _ => directional(direction, index, buttons).unwrap_or(index),
        };
        self.current = Some(buttons[target].0.clone());
    }
}

fn directional(direction: Direction, from: usize, buttons: &[(String, Rect)]) -> Option<usize> {
    let (dx, dy) = match direction {
        Direction::Up => (0.0, -1.0),
        Direction::Down => (0.0, 1.0),
        Direction::Left => (-1.0, 0.0),
        Direction::Right => (1.0, 0.0),
        _ => return None,
    };
    let (cx, cy) = buttons[from].1.center();
    // (distance along the direction, distance off the axis)
    let measure = |i: usize| {
        let (x, y) = buttons[i].1.center();
        let (vx, vy) = (x - cx, y - cy);
        (vx * dx + vy * dy, (vx * dy - vy * dx).abs())
    };
    let others = || (0..buttons.len()).filter(|&i| i != from);
    let ahead = others().filter(|&i| measure(i).0 > 1e-3).min_by(|&a, &b| {
        let score = |i: usize| {
            let (along, across) = measure(i);
            along + 2.0 * across
        };
        score(a).total_cmp(&score(b))
    });
    // Nothing ahead: wrap to the farthest button behind, preferring the one most in line.
    ahead.or_else(|| {
        others().min_by(|&a, &b| {
            let (aa, ac) = measure(a);
            let (ba, bc) = measure(b);
            aa.total_cmp(&ba).then(ac.total_cmp(&bc))
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn column(n: usize) -> Vec<(String, Rect)> {
        (0..n)
            .map(|i| {
                (
                    format!("b{i}"),
                    Rect::new(0.0, i as f32 * 50.0, 100.0, 40.0),
                )
            })
            .collect()
    }

    #[test]
    fn down_and_up_walk_a_column_and_wrap() {
        let buttons = column(3);
        let mut focus = Focus::default();
        focus.ensure(&buttons);
        assert_eq!(focus.current(), Some("b0"));
        focus.navigate(Direction::Down, &buttons);
        focus.navigate(Direction::Down, &buttons);
        assert_eq!(focus.current(), Some("b2"));
        focus.navigate(Direction::Down, &buttons);
        assert_eq!(focus.current(), Some("b0"), "wraps to the top");
        focus.navigate(Direction::Up, &buttons);
        assert_eq!(focus.current(), Some("b2"), "wraps to the bottom");
    }

    #[test]
    fn left_right_prefer_the_button_in_line() {
        let buttons = vec![
            ("a".to_string(), Rect::new(0.0, 0.0, 50.0, 40.0)),
            ("b".to_string(), Rect::new(100.0, 0.0, 50.0, 40.0)),
            ("c".to_string(), Rect::new(60.0, 200.0, 50.0, 40.0)),
        ];
        let mut focus = Focus::default();
        focus.set("a");
        focus.navigate(Direction::Right, &buttons);
        assert_eq!(focus.current(), Some("b"), "b is in line, c is far below");
    }

    #[test]
    fn tab_order_follows_the_tree() {
        let buttons = column(3);
        let mut focus = Focus::default();
        focus.set("b1");
        focus.navigate(Direction::Next, &buttons);
        assert_eq!(focus.current(), Some("b2"));
        focus.navigate(Direction::Previous, &buttons);
        focus.navigate(Direction::Previous, &buttons);
        assert_eq!(focus.current(), Some("b0"));
    }

    #[test]
    fn focus_recovers_when_its_button_disappears() {
        let mut focus = Focus::default();
        focus.set("gone");
        focus.ensure(&column(2));
        assert_eq!(focus.current(), Some("b0"));
        focus.ensure(&[]);
        assert_eq!(focus.current(), None);
        focus.navigate(Direction::Down, &[]);
    }
}
