// Ayame Editor — modal registry facade.
//
// The implementation lives with setModalOpen so visibility, LIFO ordering,
// accessibility state, Escape, and backdrop dismissal share one owner.
export { anyModalOpen, closeTopModal, initModalRegistry, registerModal } from "./dom.js";
