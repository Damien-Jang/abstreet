use abstutil;
use ezgui::{Canvas, GfxCtx, InputResult, Menu, TextBox, UserInput};
use map_model::Map;
use sim::{Neighborhood, Tick};
use std::any::Any;
use std::collections::VecDeque;

pub struct Wizard {
    alive: bool,
    tb: Option<TextBox>,
    menu: Option<Menu<Box<Cloneable>>>,

    // In the order of queries made
    confirmed_state: Vec<Box<Cloneable>>,
}

impl Wizard {
    pub fn new() -> Wizard {
        Wizard {
            alive: true,
            tb: None,
            menu: None,
            confirmed_state: Vec::new(),
        }
    }

    pub fn draw(&self, g: &mut GfxCtx, canvas: &Canvas) {
        if let Some(ref menu) = self.menu {
            menu.draw(g, canvas);
        }
        if let Some(ref tb) = self.tb {
            tb.draw(g, canvas);
        }
    }

    pub fn wrap<'a>(&'a mut self, input: &'a mut UserInput, map: &'a Map) -> WrappedWizard<'a> {
        assert!(self.alive);

        let ready_results = VecDeque::from(self.confirmed_state.clone());
        WrappedWizard {
            wizard: self,
            input,
            map,
            ready_results,
        }
    }

    pub fn aborted(&self) -> bool {
        !self.alive
    }

    // The caller can ask for any type at any time
    pub fn current_menu_choice<R: 'static + Cloneable>(&self) -> Option<&R> {
        if let Some(ref menu) = self.menu {
            let item: &R = menu.current_choice().as_any().downcast_ref::<R>()?;
            return Some(item);
        }
        None
    }

    fn input_with_text_box<R: Cloneable>(
        &mut self,
        query: &str,
        input: &mut UserInput,
        parser: Box<Fn(String) -> Option<R>>,
    ) -> Option<R> {
        assert!(self.alive);

        // Otherwise, we try to use one event for two inputs potentially
        if input.has_been_consumed() {
            return None;
        }

        if self.tb.is_none() {
            self.tb = Some(TextBox::new(query));
        }

        match self.tb.as_mut().unwrap().event(input) {
            InputResult::StillActive => None,
            InputResult::Canceled => {
                self.alive = false;
                None
            }
            InputResult::Done(line, _) => {
                self.tb = None;
                if let Some(result) = parser(line.clone()) {
                    Some(result)
                } else {
                    warn!("Invalid input {}", line);
                    None
                }
            }
        }
    }
}

// Lives only for one frame -- bundles up temporary things like UserInput and statefully serve
// prior results.
pub struct WrappedWizard<'a> {
    wizard: &'a mut Wizard,
    input: &'a mut UserInput,
    // TODO a workflow needs the map name. fine?
    pub map: &'a Map,

    // The downcasts are safe iff the queries made to the wizard are deterministic.
    ready_results: VecDeque<Box<Cloneable>>,
}

impl<'a> WrappedWizard<'a> {
    pub fn input_something<R: 'static + Clone + Cloneable>(
        &mut self,
        query: &str,
        parser: Box<Fn(String) -> Option<R>>,
    ) -> Option<R> {
        if !self.ready_results.is_empty() {
            let first = self.ready_results.pop_front().unwrap();
            let item: &R = first.as_any().downcast_ref::<R>().unwrap();
            return Some(item.clone());
        }
        if let Some(obj) = self.wizard.input_with_text_box(query, self.input, parser) {
            self.wizard.confirmed_state.push(Box::new(obj.clone()));
            Some(obj)
        } else {
            None
        }
    }

    // Conveniently predefined things
    pub fn input_string(&mut self, query: &str) -> Option<String> {
        self.input_something(query, Box::new(|line| Some(line)))
    }

    pub fn input_usize(&mut self, query: &str) -> Option<usize> {
        self.input_something(query, Box::new(|line| line.parse::<usize>().ok()))
    }

    pub fn input_tick(&mut self, query: &str) -> Option<Tick> {
        self.input_something(query, Box::new(|line| Tick::parse(&line)))
    }

    pub fn input_percent(&mut self, query: &str) -> Option<f64> {
        self.input_something(
            query,
            Box::new(|line| {
                line.parse::<f64>().ok().and_then(|num| {
                    if num >= 0.0 && num <= 1.0 {
                        Some(num)
                    } else {
                        None
                    }
                })
            }),
        )
    }

    pub fn choose_something<R: 'static + Clone + Cloneable>(
        &mut self,
        query: &str,
        choices_generator: Box<Fn() -> Vec<(String, R)>>,
    ) -> Option<(String, R)> {
        if !self.ready_results.is_empty() {
            let first = self.ready_results.pop_front().unwrap();
            // We have to downcast twice! \o/
            let pair: &(String, Box<Cloneable>) = first
                .as_any()
                .downcast_ref::<(String, Box<Cloneable>)>()
                .unwrap();
            let item: &R = pair.1.as_any().downcast_ref::<R>().unwrap();
            return Some((pair.0.to_string(), item.clone()));
        }

        if self.wizard.menu.is_none() {
            let choices: Vec<(String, R)> = choices_generator();
            let boxed_choices: Vec<(String, Box<Cloneable>)> = choices
                .iter()
                .map(|(s, item)| (s.to_string(), item.clone_box()))
                .collect();
            self.wizard.menu = Some(Menu::new(query, boxed_choices));
        }

        if let Some((choice, item)) =
            input_with_menu(&mut self.wizard.menu, &mut self.wizard.alive, self.input)
        {
            self.wizard
                .confirmed_state
                .push(Box::new((choice.to_string(), item.clone())));
            let downcasted_item: &R = item.as_any().downcast_ref::<R>().unwrap();
            Some((choice, downcasted_item.clone()))
        } else {
            None
        }
    }

    // Conveniently predefined things
    pub fn choose_string(&mut self, query: &str, choices: Vec<&str>) -> Option<String> {
        // Clone the choices outside of the closure to get around the fact that choices_generator's
        // lifetime isn't correctly specified.
        let copied_choices: Vec<(String, ())> =
            choices.into_iter().map(|s| (s.to_string(), ())).collect();
        self.choose_something(query, Box::new(move || copied_choices.clone()))
            .map(|(s, _)| s)
    }

    pub fn choose_neighborhood(&mut self, query: &str) -> Option<String> {
        // The closure's lifetime is the same as WrappedWizard (it doesn't live past the call to
        // choose_something), but I'm not quite sure how to express that yet, so clone the
        // map_name.
        let map_name = self.map.get_name().to_string();
        self.choose_something::<Neighborhood>(
            query,
            Box::new(move || abstutil::load_all_objects("neighborhoods", &map_name)),
        ).map(|(n, _)| n)
    }
}

// The caller initializes the menu, if needed. Pass in Option that must be Some().
// Bit weird to be a free function, but need to borrow a different menu and also the alive bit.
fn input_with_menu<T: Clone>(
    menu: &mut Option<Menu<T>>,
    alive: &mut bool,
    input: &mut UserInput,
) -> Option<(String, T)> {
    assert!(*alive);

    // Otherwise, we try to use one event for two inputs potentially
    if input.has_been_consumed() {
        return None;
    }

    match menu.as_mut().unwrap().event(input) {
        InputResult::Canceled => {
            *menu = None;
            *alive = false;
            None
        }
        InputResult::StillActive => None,
        InputResult::Done(name, poly) => {
            *menu = None;
            Some((name, poly))
        }
    }
}

// Trick to make a cloneable Any from
// https://stackoverflow.com/questions/30353462/how-to-clone-a-struct-storing-a-boxed-trait-object/30353928#30353928.

pub trait Cloneable: CloneableImpl {}

pub trait CloneableImpl {
    fn clone_box(&self) -> Box<Cloneable>;
    fn as_any(&self) -> &Any;
}

impl<T> CloneableImpl for T
where
    T: 'static + Cloneable + Clone,
{
    fn clone_box(&self) -> Box<Cloneable> {
        Box::new(self.clone())
    }

    fn as_any(&self) -> &Any {
        self
    }
}

impl Clone for Box<Cloneable> {
    fn clone(&self) -> Box<Cloneable> {
        self.clone_box()
    }
}

impl Cloneable for String {}
impl Cloneable for usize {}
impl Cloneable for Tick {}
impl Cloneable for f64 {}
impl Cloneable for () {}
impl Cloneable for Neighborhood {}
impl Cloneable for (String, Box<Cloneable>) {}
