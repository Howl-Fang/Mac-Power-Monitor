use ratatui::style::{Color, Modifier, Style};

#[derive(Clone, Copy)]
pub struct Theme {
    pub fg: Color,
    pub dim: Color,
    pub border: Color,
    pub accent: Color,
    pub green: Color,
    pub yellow: Color,
    pub orange: Color,
    pub red: Color,
    pub purple: Color,
    pub blue: Color,
    pub no_color: bool,
}

impl Theme {
    pub fn new(no_color: bool) -> Self {
        if no_color {
            Self {
                fg: Color::Reset,
                dim: Color::Reset,
                border: Color::Reset,
                accent: Color::Reset,
                green: Color::Reset,
                yellow: Color::Reset,
                orange: Color::Reset,
                red: Color::Reset,
                purple: Color::Reset,
                blue: Color::Reset,
                no_color: true,
            }
        } else {
            Self {
                fg: Color::Rgb(218, 222, 228),
                dim: Color::Rgb(130, 138, 148),
                border: Color::Rgb(80, 92, 105),
                accent: Color::Rgb(80, 195, 255),
                green: Color::Rgb(90, 215, 130),
                yellow: Color::Rgb(255, 205, 70),
                orange: Color::Rgb(255, 150, 60),
                red: Color::Rgb(255, 95, 95),
                purple: Color::Rgb(190, 130, 255),
                blue: Color::Rgb(90, 155, 255),
                no_color: false,
            }
        }
    }

    pub fn text(&self) -> Style {
        Style::default().fg(self.fg)
    }

    pub fn dim_text(&self) -> Style {
        Style::default().fg(self.dim)
    }

    pub fn title(&self) -> Style {
        Style::default()
            .fg(self.accent)
            .add_modifier(Modifier::BOLD)
    }

    pub fn border_style(&self) -> Style {
        Style::default().fg(self.border)
    }

    pub fn selected(&self) -> Style {
        if self.no_color {
            Style::default().add_modifier(Modifier::REVERSED | Modifier::BOLD)
        } else {
            Style::default()
                .fg(Color::Rgb(10, 14, 18))
                .bg(self.accent)
                .add_modifier(Modifier::BOLD)
        }
    }

    pub fn warn(&self) -> Style {
        Style::default()
            .fg(self.yellow)
            .add_modifier(Modifier::BOLD)
    }

    pub fn error(&self) -> Style {
        Style::default().fg(self.red).add_modifier(Modifier::BOLD)
    }
}
