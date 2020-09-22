use crate::{
    dom::Dispatch,
    html,
    mt_dom::{
        Callback,
        NodeIdx,
    },
    Attribute,
};
use std::{
    collections::HashMap,
    ops::Deref,
    sync::Mutex,
};
use wasm_bindgen::{
    closure::Closure,
    JsCast,
};
use web_sys::{
    self,
    Element,
    EventTarget,
    HtmlInputElement,
    HtmlTextAreaElement,
    Node,
    Text,
};

// Used to uniquely identify elements that contain closures so that the DomUpdater can
// look them up by their unique id.
// When the DomUpdater sees that the element no longer exists it will drop all of it's
// Rc'd Closures for those events.
use lazy_static::lazy_static;
lazy_static! {
    /// This is the value of the data-sauron-vdom-id.
    static ref DATA_SAURON_VDOM_ID_VALUE: Mutex<u32> = Mutex::new(0);
}

fn create_unique_identifier() -> u32 {
    let mut elem_unique_id = DATA_SAURON_VDOM_ID_VALUE
        .lock()
        .expect("Unable to obtain lock");
    *elem_unique_id += 1;
    *elem_unique_id
}

pub(crate) const DATA_SAURON_VDOM_ID: &str = "data-sauron-vdom-id";

/// Closures that we are holding on to to make sure that they don't get invalidated after a
/// VirtualNode is dropped.
///
/// The u32 is a unique identifier that is associated with the DOM element that this closure is
/// attached to.
///
pub type ActiveClosure =
    HashMap<u32, Vec<(&'static str, Closure<dyn FnMut(web_sys::Event)>)>>;

/// A node along with all of the closures that were created for that
/// node's events and all of it's child node's events.
#[derive(Debug)]
pub struct CreatedNode<T> {
    /// A `Node` or `Element` that was created from a `Node`
    pub node: T,
    pub(crate) closures: ActiveClosure,
}

impl<T> CreatedNode<T> {
    /// create a simple node with no closure attache
    pub fn without_closures<N: Into<T>>(node: N) -> Self {
        CreatedNode {
            node: node.into(),
            closures: HashMap::with_capacity(0),
        }
    }

    /// create a text node
    pub fn create_text_node(txt: &str) -> Text {
        crate::document().create_text_node(txt)
    }

    /// create an element node
    pub fn create_dom_node<DSP, MSG>(
        program: &DSP,
        vnode: &crate::Node<MSG>,
        node_idx: &mut Option<NodeIdx>,
    ) -> CreatedNode<Node>
    where
        MSG: 'static,
        DSP: Clone + Dispatch<MSG> + 'static,
    {
        Self::create_dom_node_opt(Some(program), vnode, node_idx)
    }

    /// Create and return a `CreatedNode` instance (containing a DOM `Node`
    /// together with potentially related closures) for this virtual node.
    ///
    /// TODO: Optimization for apply_patches::find_nodes
    /// Keep track of the Node with their corresponding NodeIdx, so as not
    /// to traverse all of them to find the node.
    /// We can maintain a HashMap<NodeIdx, web_sys::Node>
    pub fn create_dom_node_opt<DSP, MSG>(
        program: Option<&DSP>,
        vnode: &crate::Node<MSG>,
        node_idx: &mut Option<NodeIdx>,
    ) -> CreatedNode<Node>
    where
        MSG: 'static,
        DSP: Clone + Dispatch<MSG> + 'static,
    {
        match vnode {
            crate::Node::Text(txt) => {
                CreatedNode::without_closures(Self::create_text_node(txt))
            }
            crate::Node::Element(element_node) => {
                let created_element: CreatedNode<Node> =
                    Self::create_element_node(program, element_node, node_idx)
                        .into();
                created_element
            }
        }
    }

    /// set the element attribute
    pub fn set_element_attributes<DSP, MSG>(
        program: Option<&DSP>,
        closures: &mut ActiveClosure,
        element: &Element,
        attrs: &[&Attribute<MSG>],
    ) where
        MSG: 'static,
        DSP: Clone + Dispatch<MSG> + 'static,
    {
        let attrs = mt_dom::merge_attributes_of_same_name(attrs);
        for att in attrs {
            Self::set_element_attribute(program, closures, element, &att);
        }
    }

    /// set the element attribute
    pub fn set_element_attribute<DSP, MSG>(
        program: Option<&DSP>,
        closures: &mut ActiveClosure,
        element: &Element,
        attr: &Attribute<MSG>,
    ) where
        MSG: 'static,
        DSP: Clone + Dispatch<MSG> + 'static,
    {
        let (callbacks, plain_values, func_values) =
            html::attributes::partition_callbacks_from_plain_and_func_calls(
                attr,
            );

        // set simple values
        if let Some(merged_plain_values) =
            html::attributes::merge_plain_attributes_values(&plain_values)
        {
            if let Some(ref namespace) = attr.namespace() {
                // Warning NOTE: set_attribute_ns should only be called
                // when you meant to use a namespace
                // using this with None will error in the browser with:
                // NamespaceError: An attempt was made to create or change an object in a way which is incorrect with regard to namespaces
                element
                    .set_attribute_ns(
                        Some(namespace),
                        attr.name(),
                        &merged_plain_values,
                    )
                    .expect("Set element attribute_ns in create element");
            } else {
                match *attr.name() {
                    "value" => {
                        if let Some(input) =
                            element.dyn_ref::<HtmlInputElement>()
                        {
                            input.set_value(&merged_plain_values);
                        } else if let Some(textarea) =
                            element.dyn_ref::<HtmlTextAreaElement>()
                        {
                            textarea.set_value(&merged_plain_values);
                        }
                    }
                    "checked" => {
                        if let Some(input) =
                            element.dyn_ref::<HtmlInputElement>()
                        {
                            let checked = plain_values
                                .first()
                                .map(|av| {
                                    av.get_simple()
                                        .expect("must be a simple value")
                                })
                                .unwrap()
                                .to_string();
                            if !checked.is_empty() {
                                input.set_checked(true);
                            }
                        }
                    }
                    _ => {
                        element
                            .set_attribute(attr.name(), &merged_plain_values)
                            .expect("Set element attribute in create element");
                    }
                }
            }
        } else {
            //if the merged attribute is blank of empty when string is trimmed
            //remove the attribute
            element
                .remove_attribute(attr.name())
                .expect("must remove attribute");
        }

        // do function calls such as set_inner_html
        if let Some(merged_func_values) =
            html::attributes::merge_plain_attributes_values(&func_values)
        {
            match *attr.name() {
                "inner_html" => element.set_inner_html(&merged_func_values),
                _ => (),
            }
        }

        // add callbacks using add_event_listener
        for callback in callbacks {
            let unique_id = create_unique_identifier();

            // set the data-sauron_vdom-id this will be read later on
            // when it's time to remove this element and its closures and event listeners
            element
                .set_attribute(DATA_SAURON_VDOM_ID, &unique_id.to_string())
                .expect("Could not set attribute on element");

            closures.insert(unique_id, vec![]);

            if let Some(program) = program {
                let event_str = attr.name();
                let current_elm: &EventTarget =
                    element.dyn_ref().expect("unable to cast to event targe");
                let closure_wrap: Closure<dyn FnMut(web_sys::Event)> =
                    create_closure_wrap(program, &callback);
                current_elm
                    .add_event_listener_with_callback(
                        event_str,
                        closure_wrap.as_ref().unchecked_ref(),
                    )
                    .expect("Unable to attached event listener");
                closures
                    .get_mut(&unique_id)
                    .expect("Unable to get closure")
                    .push((event_str, closure_wrap));
            }
        }
    }

    /// Build a DOM element by recursively creating DOM nodes for this element and it's
    /// children, it's children's children, etc.
    pub fn create_element_node<DSP, MSG>(
        program: Option<&DSP>,
        velem: &crate::Element<MSG>,
        node_idx: &mut Option<NodeIdx>,
    ) -> CreatedNode<Element>
    where
        MSG: 'static,
        DSP: Clone + Dispatch<MSG> + 'static,
    {
        let document = crate::document();

        let element = if let Some(ref namespace) = velem.namespace() {
            document
                .create_element_ns(Some(namespace), &velem.tag())
                .expect("Unable to create element")
        } else {
            document
                .create_element(&velem.tag())
                .expect("Unable to create element")
        };

        let mut closures = ActiveClosure::new();

        // TODO: attributes can be further merge here so as not to override the previous value
        // when there are more than 1 attributes of the same name in an element
        Self::set_element_attributes(
            program,
            &mut closures,
            &element,
            &velem.get_attributes().iter().collect::<Vec<_>>(),
        );

        #[cfg(feature = "with-nodeidx")]
        if let Some(node_idx) = node_idx {
            Self::set_element_attributes(
                program,
                &mut closures,
                &element,
                &[&crate::prelude::attr("node_idx", *node_idx)],
            );
        }

        let mut previous_node_was_text = false;
        for child in velem.get_children().iter() {
            match child {
                crate::Node::Text(txt) => {
                    node_idx.as_mut().map(|idx| *idx += 1);
                    let current_node: &web_sys::Node = element.as_ref();

                    // We ensure that the text siblings are patched by preventing the browser from merging
                    // neighboring text nodes. Originally inspired by some of React's work from 2016.
                    //  -> https://reactjs.org/blog/2016/04/07/react-v15.html#major-changes
                    //  -> https://github.com/facebook/react/pull/5753
                    //
                    // `mordor` one does not simply walk into mordor
                    if previous_node_was_text {
                        let separator = document.create_comment("mordor");
                        current_node
                            .append_child(separator.as_ref())
                            .expect("Unable to append child");
                    }

                    current_node
                        .append_child(&Self::create_text_node(&txt))
                        .expect("Unable to append text node");

                    previous_node_was_text = true;
                }
                crate::Node::Element(element_node) => {
                    node_idx.as_mut().map(|idx| *idx += 1);
                    previous_node_was_text = false;

                    let child = Self::create_element_node(
                        program,
                        element_node,
                        node_idx,
                    );
                    let child_elem: Element = child.node;
                    closures.extend(child.closures);

                    element
                        .append_child(&child_elem)
                        .expect("Unable to append element node");
                }
            }
        }

        CreatedNode {
            node: element,
            closures,
        }
    }
}

/// This wrap into a closure the function that is dispatched when the event is triggered.
pub(crate) fn create_closure_wrap<DSP, MSG>(
    program: &DSP,
    callback: &Callback<crate::Event, MSG>,
) -> Closure<dyn FnMut(web_sys::Event)>
where
    MSG: 'static,
    DSP: Clone + Dispatch<MSG> + 'static,
{
    let callback_clone = callback.clone();
    // TODO: use a weak pointer here
    // let program_weak = Rc::downgrade(&program)
    let program_clone = program.clone();

    Closure::wrap(Box::new(move |event: web_sys::Event| {
        // Note:
        // calling `event.stop_propagation()` to the containers of this element to have
        // a more fine grain control and expected results,
        // and for most cases this is what we want. We don't want the containing div of a button
        // also receives that click event.
        //event.stop_propagation();
        //
        // Notes:
        // - calling event.prevent_default() prevents the reloading the page in href links, which is what we
        // want mostly in an SPA app
        // - calling event.prevent_default() prevent InputEvent to trigger when KeyPressEvent is
        // also one of the event callback
        // event.prevent_default();
        let msg = callback_clone.emit(event);
        program_clone.dispatch(msg);
    }))
}

impl From<CreatedNode<Element>> for CreatedNode<Node> {
    fn from(other: CreatedNode<Element>) -> CreatedNode<Node> {
        CreatedNode {
            node: other.node.into(),
            closures: other.closures,
        }
    }
}

impl<T> Deref for CreatedNode<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.node
    }
}
