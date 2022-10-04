use crate::contacts_popover;
use call::{ActiveCall, ParticipantLocation};
use client::{Authenticate, PeerId};
use clock::ReplicaId;
use contacts_popover::ContactsPopover;
use gpui::{
    actions,
    color::Color,
    elements::*,
    geometry::{rect::RectF, vector::vec2f, PathBuilder},
    json::{self, ToJson},
    Border, CursorStyle, Entity, ImageData, MouseButton, MutableAppContext, RenderContext,
    Subscription, View, ViewContext, ViewHandle, WeakViewHandle,
};
use settings::Settings;
use std::{ops::Range, sync::Arc};
use theme::Theme;
use workspace::{FollowNextCollaborator, ToggleFollow, Workspace};

actions!(
    contacts_titlebar_item,
    [ToggleContactsPopover, ShareProject]
);

pub fn init(cx: &mut MutableAppContext) {
    cx.add_action(CollabTitlebarItem::toggle_contacts_popover);
    cx.add_action(CollabTitlebarItem::share_project);
}

pub struct CollabTitlebarItem {
    workspace: WeakViewHandle<Workspace>,
    contacts_popover: Option<ViewHandle<ContactsPopover>>,
    _subscriptions: Vec<Subscription>,
}

impl Entity for CollabTitlebarItem {
    type Event = ();
}

impl View for CollabTitlebarItem {
    fn ui_name() -> &'static str {
        "CollabTitlebarItem"
    }

    fn render(&mut self, cx: &mut RenderContext<Self>) -> ElementBox {
        let workspace = if let Some(workspace) = self.workspace.upgrade(cx) {
            workspace
        } else {
            return Empty::new().boxed();
        };

        let theme = cx.global::<Settings>().theme.clone();
        let project = workspace.read(cx).project().read(cx);

        let mut container = Flex::row();
        if workspace.read(cx).client().status().borrow().is_connected() {
            if project.is_shared()
                || project.is_remote()
                || ActiveCall::global(cx).read(cx).room().is_none()
            {
                container.add_child(self.render_toggle_contacts_button(&theme, cx));
            } else {
                container.add_child(self.render_share_button(&theme, cx));
            }
        }
        container.add_children(self.render_collaborators(&workspace, &theme, cx));
        container.add_children(self.render_current_user(&workspace, &theme, cx));
        container.add_children(self.render_connection_status(&workspace, cx));
        container.boxed()
    }
}

impl CollabTitlebarItem {
    pub fn new(workspace: &ViewHandle<Workspace>, cx: &mut ViewContext<Self>) -> Self {
        let active_call = ActiveCall::global(cx);
        let mut subscriptions = Vec::new();
        subscriptions.push(cx.observe(workspace, |_, _, cx| cx.notify()));
        subscriptions.push(cx.observe(&active_call, |_, _, cx| cx.notify()));
        Self {
            workspace: workspace.downgrade(),
            contacts_popover: None,
            _subscriptions: subscriptions,
        }
    }

    fn share_project(&mut self, _: &ShareProject, cx: &mut ViewContext<Self>) {
        if let Some(workspace) = self.workspace.upgrade(cx) {
            if let Some(room) = ActiveCall::global(cx).read(cx).room() {
                let room_id = room.read(cx).id();
                let project = workspace.read(cx).project().clone();
                project
                    .update(cx, |project, cx| project.share(room_id, cx))
                    .detach_and_log_err(cx);
            }
        }
    }

    fn toggle_contacts_popover(&mut self, _: &ToggleContactsPopover, cx: &mut ViewContext<Self>) {
        match self.contacts_popover.take() {
            Some(_) => {}
            None => {
                if let Some(workspace) = self.workspace.upgrade(cx) {
                    let project = workspace.read(cx).project().clone();
                    let user_store = workspace.read(cx).user_store().clone();
                    let view = cx.add_view(|cx| ContactsPopover::new(project, user_store, cx));
                    cx.focus(&view);
                    cx.subscribe(&view, |this, _, event, cx| {
                        match event {
                            contacts_popover::Event::Dismissed => {
                                this.contacts_popover = None;
                            }
                        }

                        cx.notify();
                    })
                    .detach();
                    self.contacts_popover = Some(view);
                }
            }
        }
        cx.notify();
    }

    fn render_toggle_contacts_button(
        &self,
        theme: &Theme,
        cx: &mut RenderContext<Self>,
    ) -> ElementBox {
        let titlebar = &theme.workspace.titlebar;
        Stack::new()
            .with_child(
                MouseEventHandler::<ToggleContactsPopover>::new(0, cx, |state, _| {
                    let style = titlebar
                        .toggle_contacts_button
                        .style_for(state, self.contacts_popover.is_some());
                    Svg::new("icons/plus_8.svg")
                        .with_color(style.color)
                        .constrained()
                        .with_width(style.icon_width)
                        .aligned()
                        .constrained()
                        .with_width(style.button_width)
                        .with_height(style.button_width)
                        .contained()
                        .with_style(style.container)
                        .boxed()
                })
                .with_cursor_style(CursorStyle::PointingHand)
                .on_click(MouseButton::Left, |_, cx| {
                    cx.dispatch_action(ToggleContactsPopover);
                })
                .aligned()
                .boxed(),
            )
            .with_children(self.contacts_popover.as_ref().map(|popover| {
                Overlay::new(
                    ChildView::new(popover)
                        .contained()
                        .with_margin_top(titlebar.height)
                        .with_margin_right(-titlebar.toggle_contacts_button.default.button_width)
                        .boxed(),
                )
                .with_fit_mode(OverlayFitMode::SwitchAnchor)
                .with_anchor_corner(AnchorCorner::BottomLeft)
                .boxed()
            }))
            .boxed()
    }

    fn render_share_button(&self, theme: &Theme, cx: &mut RenderContext<Self>) -> ElementBox {
        enum Share {}

        let titlebar = &theme.workspace.titlebar;
        MouseEventHandler::<Share>::new(0, cx, |state, _| {
            let style = titlebar.share_button.style_for(state, false);
            Label::new("Share".into(), style.text.clone())
                .contained()
                .with_style(style.container)
                .boxed()
        })
        .with_cursor_style(CursorStyle::PointingHand)
        .on_click(MouseButton::Left, |_, cx| cx.dispatch_action(ShareProject))
        .with_tooltip::<Share, _>(
            0,
            "Share project with call participants".into(),
            None,
            theme.tooltip.clone(),
            cx,
        )
        .aligned()
        .boxed()
    }

    fn render_collaborators(
        &self,
        workspace: &ViewHandle<Workspace>,
        theme: &Theme,
        cx: &mut RenderContext<Self>,
    ) -> Vec<ElementBox> {
        let active_call = ActiveCall::global(cx);
        if let Some(room) = active_call.read(cx).room().cloned() {
            let project = workspace.read(cx).project().read(cx);
            let project_id = project.remote_id();
            let mut collaborators = project
                .collaborators()
                .values()
                .cloned()
                .collect::<Vec<_>>();
            collaborators.sort_by_key(|collaborator| collaborator.replica_id);
            collaborators
                .into_iter()
                .filter_map(|collaborator| {
                    let participant = room
                        .read(cx)
                        .remote_participants()
                        .get(&collaborator.peer_id)?;
                    let user = participant.user.clone();
                    let is_active = project_id.map_or(false, |project_id| {
                        participant.location == ParticipantLocation::Project { project_id }
                    });
                    Some(self.render_avatar(
                        user.avatar.clone()?,
                        collaborator.replica_id,
                        Some((collaborator.peer_id, &user.github_login)),
                        is_active,
                        workspace,
                        theme,
                        cx,
                    ))
                })
                .collect()
        } else {
            Default::default()
        }
    }

    fn render_current_user(
        &self,
        workspace: &ViewHandle<Workspace>,
        theme: &Theme,
        cx: &mut RenderContext<Self>,
    ) -> Option<ElementBox> {
        let user = workspace.read(cx).user_store().read(cx).current_user();
        let replica_id = workspace.read(cx).project().read(cx).replica_id();
        let status = *workspace.read(cx).client().status().borrow();
        if let Some(avatar) = user.and_then(|user| user.avatar.clone()) {
            Some(self.render_avatar(avatar, replica_id, None, true, workspace, theme, cx))
        } else if matches!(status, client::Status::UpgradeRequired) {
            None
        } else {
            Some(
                MouseEventHandler::<Authenticate>::new(0, cx, |state, _| {
                    let style = theme
                        .workspace
                        .titlebar
                        .sign_in_prompt
                        .style_for(state, false);
                    Label::new("Sign in".to_string(), style.text.clone())
                        .contained()
                        .with_style(style.container)
                        .boxed()
                })
                .on_click(MouseButton::Left, |_, cx| cx.dispatch_action(Authenticate))
                .with_cursor_style(CursorStyle::PointingHand)
                .aligned()
                .boxed(),
            )
        }
    }

    fn render_avatar(
        &self,
        avatar: Arc<ImageData>,
        replica_id: ReplicaId,
        peer: Option<(PeerId, &str)>,
        is_active: bool,
        workspace: &ViewHandle<Workspace>,
        theme: &Theme,
        cx: &mut RenderContext<Self>,
    ) -> ElementBox {
        let replica_color = theme.editor.replica_selection_style(replica_id).cursor;
        let is_followed = peer.map_or(false, |(peer_id, _)| {
            workspace.read(cx).is_following(peer_id)
        });
        let mut avatar_style;

        if is_active {
            avatar_style = theme.workspace.titlebar.avatar;
        } else {
            avatar_style = theme.workspace.titlebar.inactive_avatar;
        }

        if is_followed {
            avatar_style.border = Border::all(1.0, replica_color);
        }

        let content = Stack::new()
            .with_child(
                Image::new(avatar)
                    .with_style(avatar_style)
                    .constrained()
                    .with_width(theme.workspace.titlebar.avatar_width)
                    .aligned()
                    .boxed(),
            )
            .with_child(
                AvatarRibbon::new(replica_color)
                    .constrained()
                    .with_width(theme.workspace.titlebar.avatar_ribbon.width)
                    .with_height(theme.workspace.titlebar.avatar_ribbon.height)
                    .aligned()
                    .bottom()
                    .boxed(),
            )
            .constrained()
            .with_width(theme.workspace.titlebar.avatar_width)
            .contained()
            .with_margin_left(theme.workspace.titlebar.avatar_margin)
            .boxed();

        if let Some((peer_id, peer_github_login)) = peer {
            MouseEventHandler::<ToggleFollow>::new(replica_id.into(), cx, move |_, _| content)
                .with_cursor_style(CursorStyle::PointingHand)
                .on_click(MouseButton::Left, move |_, cx| {
                    cx.dispatch_action(ToggleFollow(peer_id))
                })
                .with_tooltip::<ToggleFollow, _>(
                    peer_id.0 as usize,
                    if is_followed {
                        format!("Unfollow {}", peer_github_login)
                    } else {
                        format!("Follow {}", peer_github_login)
                    },
                    Some(Box::new(FollowNextCollaborator)),
                    theme.tooltip.clone(),
                    cx,
                )
                .boxed()
        } else {
            content
        }
    }

    fn render_connection_status(
        &self,
        workspace: &ViewHandle<Workspace>,
        cx: &mut RenderContext<Self>,
    ) -> Option<ElementBox> {
        let theme = &cx.global::<Settings>().theme;
        match &*workspace.read(cx).client().status().borrow() {
            client::Status::ConnectionError
            | client::Status::ConnectionLost
            | client::Status::Reauthenticating { .. }
            | client::Status::Reconnecting { .. }
            | client::Status::ReconnectionError { .. } => Some(
                Container::new(
                    Align::new(
                        ConstrainedBox::new(
                            Svg::new("icons/cloud_slash_12.svg")
                                .with_color(theme.workspace.titlebar.offline_icon.color)
                                .boxed(),
                        )
                        .with_width(theme.workspace.titlebar.offline_icon.width)
                        .boxed(),
                    )
                    .boxed(),
                )
                .with_style(theme.workspace.titlebar.offline_icon.container)
                .boxed(),
            ),
            client::Status::UpgradeRequired => Some(
                Label::new(
                    "Please update Zed to collaborate".to_string(),
                    theme.workspace.titlebar.outdated_warning.text.clone(),
                )
                .contained()
                .with_style(theme.workspace.titlebar.outdated_warning.container)
                .aligned()
                .boxed(),
            ),
            _ => None,
        }
    }
}

pub struct AvatarRibbon {
    color: Color,
}

impl AvatarRibbon {
    pub fn new(color: Color) -> AvatarRibbon {
        AvatarRibbon { color }
    }
}

impl Element for AvatarRibbon {
    type LayoutState = ();

    type PaintState = ();

    fn layout(
        &mut self,
        constraint: gpui::SizeConstraint,
        _: &mut gpui::LayoutContext,
    ) -> (gpui::geometry::vector::Vector2F, Self::LayoutState) {
        (constraint.max, ())
    }

    fn paint(
        &mut self,
        bounds: gpui::geometry::rect::RectF,
        _: gpui::geometry::rect::RectF,
        _: &mut Self::LayoutState,
        cx: &mut gpui::PaintContext,
    ) -> Self::PaintState {
        let mut path = PathBuilder::new();
        path.reset(bounds.lower_left());
        path.curve_to(
            bounds.origin() + vec2f(bounds.height(), 0.),
            bounds.origin(),
        );
        path.line_to(bounds.upper_right() - vec2f(bounds.height(), 0.));
        path.curve_to(bounds.lower_right(), bounds.upper_right());
        path.line_to(bounds.lower_left());
        cx.scene.push_path(path.build(self.color, None));
    }

    fn dispatch_event(
        &mut self,
        _: &gpui::Event,
        _: RectF,
        _: RectF,
        _: &mut Self::LayoutState,
        _: &mut Self::PaintState,
        _: &mut gpui::EventContext,
    ) -> bool {
        false
    }

    fn rect_for_text_range(
        &self,
        _: Range<usize>,
        _: RectF,
        _: RectF,
        _: &Self::LayoutState,
        _: &Self::PaintState,
        _: &gpui::MeasurementContext,
    ) -> Option<RectF> {
        None
    }

    fn debug(
        &self,
        bounds: gpui::geometry::rect::RectF,
        _: &Self::LayoutState,
        _: &Self::PaintState,
        _: &gpui::DebugContext,
    ) -> gpui::json::Value {
        json::json!({
            "type": "AvatarRibbon",
            "bounds": bounds.to_json(),
            "color": self.color.to_json(),
        })
    }
}
