function notes(input, add, remove) {
    let items = input.storage?.items ?? [];
    let nextId = input.storage?.nextId ?? 1;
    const filled = add && items.length === 7;
    if (add) {
        items = [{id: "note-" + nextId++, text: add}, ...items].slice(0, 8);
    } else if (remove) {
        items = items.filter(item => item.id !== remove);
    }
    return {
        status: {
            state: items.length ? "running" : "idle",
            label: "便签 " + items.length + " / 8",
            fields: [{key: "note", label: "新便签", max: 80}],
            actions: [{id: "add", label: "添加"}],
            items: items.map(item => {
                const chars = [...item.text];
                return {
                    label: chars.slice(0, 28).join(""),
                    detail: chars.slice(28).join("").trim() || null,
                    action: item.id,
                    copy: item.text
                };
            })
        },
        storage: {items, nextId},
        notice: filled
            ? {id: "notes-full", title: "便签已满", body: "已记录 8 条便签。"}
            : null
    };
}

function refresh(input) {
    return notes(input);
}

function action(input) {
    return input.action.id === "add"
        ? notes(input, input.action.form.note.trim())
        : notes(input, undefined, input.action.id);
}

function event(input) {
    return notes(input, input.event.name === "focus.finished" ? "完成一次专注" : undefined);
}
