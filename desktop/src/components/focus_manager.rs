use std::{collections::HashMap, fmt::Display, rc::Rc};

use dioxus::{logger::tracing::debug, prelude::*};
use futures::StreamExt;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum FocusComponent {
    Editor,
    ModalInput,
    BrowseSearch,
}

enum Action {
    Focus(FocusComponent),
    FocusPrev,
    Register(FocusComponent, Rc<MountedData>),
    RegisterAndFocus(FocusComponent, Rc<MountedData>),
    Unregister(FocusComponent),
}

impl Display for FocusComponent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}

#[derive(Clone)]
pub struct FocusManager {
    sender: Coroutine<Action>,
}

impl FocusManager {
    pub fn new() -> Self {
        let sender = use_coroutine(|mut recv: UnboundedReceiver<Action>| async move {
            let mut registered_components: HashMap<FocusComponent, Rc<MountedData>> =
                HashMap::new();
            let mut current_component: Option<FocusComponent> = None;
            let mut prev_component: Option<FocusComponent> = None;
            while let Some(action) = recv.next().await {
                match action {
                    Action::Focus(focus_component) => {
                        focus(
                            &registered_components,
                            focus_component,
                            &mut current_component,
                            &mut prev_component,
                        )
                        .await
                    }
                    Action::FocusPrev => {
                        focus_prev(
                            &registered_components,
                            &mut current_component,
                            &mut prev_component,
                        )
                        .await
                    }
                    Action::Register(focus_component, mounted_data) => {
                        register_focus(&mut registered_components, focus_component, mounted_data)
                    }
                    Action::RegisterAndFocus(focus_component, mounted_data) => {
                        register_focus(
                            &mut registered_components,
                            focus_component.clone(),
                            mounted_data,
                        );
                        focus(
                            &registered_components,
                            focus_component,
                            &mut current_component,
                            &mut prev_component,
                        )
                        .await;
                    }
                    Action::Unregister(focus_component) => {
                        registered_components.remove(&focus_component);
                    }
                }
            }
        });
        Self { sender }
    }

    pub fn focus_prev(&self) {
        self.sender.send(Action::FocusPrev);
    }

    pub fn focus(&self, comp: FocusComponent) {
        self.sender.send(Action::Focus(comp));
    }

    pub fn register(&self, comp: FocusComponent, data: Rc<MountedData>) {
        self.sender.send(Action::Register(comp, data));
    }

    pub fn register_and_focus(&self, comp: FocusComponent, data: Rc<MountedData>) {
        self.sender.send(Action::RegisterAndFocus(comp, data));
    }

    pub fn unregister_focus(&self, comp: FocusComponent) {
        self.sender.send(Action::Unregister(comp));
    }
}

async fn focus(
    registered_components: &HashMap<FocusComponent, Rc<MountedData>>,
    comp: FocusComponent,
    current_component: &mut Option<FocusComponent>,
    prev_component: &mut Option<FocusComponent>,
) {
    debug!("About to focus on: {:?}", comp);
    if let Some(curr) = &current_component {
        // If the new component is different than the current one, we put the current as the
        // previous
        if !comp.eq(curr) {
            *prev_component = Some(curr.to_owned());
        }
    }
    if let Some(data) = registered_components.get(&comp) {
        debug!("Component found: {:?}", comp);

        let _ = data.set_focus(true).await;
        debug!("Focus set");
        *current_component = Some(comp);
    } else {
        debug!("Can't focus on: {:?}, not found", comp);
    }
}

async fn focus_prev(
    registered_components: &HashMap<FocusComponent, Rc<MountedData>>,
    current_component: &mut Option<FocusComponent>,
    prev_component: &mut Option<FocusComponent>,
) {
    if let Some(prev) = prev_component {
        debug!("Found previous component {}", prev);
        focus(
            registered_components,
            prev.to_owned(),
            current_component,
            prev_component,
        )
        .await;
    } else {
        debug!("No previous comopnent");
    }
}

fn register_focus(
    registered_components: &mut HashMap<FocusComponent, Rc<MountedData>>,
    comp: FocusComponent,
    data: Rc<MountedData>,
) {
    registered_components.insert(comp, data);
}
