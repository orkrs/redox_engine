# NET-01 — Сеть: выбор транспорта (spike) и каркас `redox_net`

- **Статус:** шаги 1–3 написаны на ветке `worktree-agent-a8fd08653da15e7f4` (ADR 0008 — предложен, подтверждает владелец; `cargo` исполнителем не запускался — проверки и спайк-замеры делает координатор через `tests/udp_spike.rs --ignored`). Отличия от спеки: `NetPlugin` вынесен в `redox_core/src/net_plugin.rs` (plugin.rs не трогали); отдельного `net_flush_system` нет, отправку делает `poll`. Не сделано: фрагментация, шифрование (`renet_netcode`), эмуляция потерь в UDP.

- **Размер:** 3 шага (S 2 дня, M, S) ≈ 6 дней. **Крейт:** новый `redox_net`. **Зависит от:** — (ADR 0008 пишется в этой задаче). **Блокирует:** NET-02, NET-03.
- **Модель:** haiku по шагам 2–3; шаг 1 (spike, таблица, ADR) — haiku, решение принимает координатор/владелец.
- Сегодня сетевой подсистемы нет: ни крейта, ни ADR (`grep -rn "net" docs/adr` пусто; в `PROGRESS.md` сеть только в строке «Далее (месяцы)»). Объём этой спеки — **основа**: транспорт, соединение, надёжные и ненадёжные каналы, сериализация сообщений, тестовый стенд. Репликация ECS — NET-02, предсказание — NET-03. **Не входит:** NAT-пробой, матчмейкинг, голосовой чат, шифрование сверх того, что даёт выбранный транспорт, выделенный сервер-оркестратор.

## Суть и допущения (владелец может поменять — до запуска шага 2)
Принятые по умолчанию (как для ADR 0007): (1) **клиент-серверная модель с авторитетным сервером** (listen-server = сервер в том же процессе, что и клиент); (2) **UDP**, с надёжными упорядоченными и ненадёжными каналами; (3) игроков до 16; (4) fixed tick сервера 30 Гц (по умолчанию), клиент рендерит каждый кадр; (5) только Windows; (6) без шифрования в v1 (локальная сеть/друзья), если транспорт даёт `netcode` с токенами — включить его; (7) сериализация `postcard` (уже в проекте для `world.bin`).

## Шаг 1 — spike: какой транспорт брать (S, 2 дня)
Оценить кандидатов **по исходникам и пробной сборке**, не по памяти (**ПРОВЕРИТЬ** наличие в реестре: `cargo search`/`cargo info <crate>`; новых загрузок спика допускает владелец — если интернета нет, оценка только по уже скачанным, и «взять» откладывается):
| Кандидат | Что проверить |
|---|---|
| `renet` + `renet_netcode` | версия, лицензия (`cargo deny`), последние релизы, надёжные/ненадёжные каналы, MTU/фрагментация, пинг/потери, чистый `std::net::UdpSocket`, без tokio? |
| `quinn` (QUIC) | тянет tokio/rustls; нужен ли async в игре; задержки; размер зависимостей |
| `laminar` | живость проекта, лицензия |
| свой минимальный слой над `std::net::UdpSocket` | объём кода (оценка), что придётся писать: ACK, повторы, фрагментация |
Пример `crates/redox_net/examples/net_spike.rs` для лучшего кандидата: сервер и клиент в одном процессе на `127.0.0.1`, обмен 1000 ненадёжных пакетов по 100 байт и 100 надёжных по 2 КБ; эмуляция потерь 10 % (если транспорт позволяет, иначе — проверить, что потери не ломают надёжный канал вставкой дропа на уровне сокета-обёртки); печать RTT, доставленных пакетов, времени.
Результат: `docs/engineering/notes/net-spike.md` (таблица кандидатов, замеры, вывод) и **`docs/adr/0008-networking.md`** (статус «предложен»; допущения 1–7 + выбор транспорта + что не делаем). Владелец подтверждает ADR до шага 2.

## Шаг 2 — каркас крейта `redox_net` (M)
Только после подтверждения ADR. Структура (названия при смене транспорта сохраняются — транспорт спрятан за трейтом):
```rust
pub trait Transport { 
    fn poll(&mut self, dt: Duration);                       // принять/отправить
    fn send(&mut self, client: ClientId, channel: Channel, data: &[u8]);
    fn broadcast(&mut self, channel: Channel, data: &[u8]);
    fn recv(&mut self, client: ClientId, channel: Channel) -> Option<Vec<u8>>;
    fn events(&mut self) -> Vec<NetEvent>;                  // Connected(id) / Disconnected(id, reason)
    fn rtt(&self, client: ClientId) -> Option<Duration>;
}
pub enum Channel { Reliable, Unreliable, Snapshots /* ненадёжный, но с отбрасыванием устаревших */ }
pub struct NetServer<T: Transport> { … }   // принимает подключения, лимит игроков, тик 30 Гц (`tick_number`), список клиентов
pub struct NetClient<T: Transport> { … }   // подключение, состояние: Connecting/Connected/Disconnected, `server_tick_estimate`
pub trait Message: Serialize + DeserializeOwned { const ID: u16; }   // postcard; заголовок: id u16 + длина
```
- Тик-синхронизация: клиент оценивает серверный тик по RTT/2 (EWMA), экспортирует `server_tick_estimate()`; тестируется на чистой функции.
- Тестовый транспорт `LoopbackTransport` (в памяти, с настройкой задержки, потерь, дублирования, перестановки по детерминированному `SplitMix64`) — **для всех тестов NET-02/03**, без сокетов.
- Реальный транспорт: адаптер выбранной библиотеки в `redox_net/src/transport_<name>.rs` (feature `udp`, по умолчанию включён).
Тесты (без сети, Loopback):
```rust
#[test] fn a_client_connects_and_the_server_sees_it() { /* события Connected */ }
#[test] fn the_player_limit_is_enforced() { /* 17-й клиент получает Disconnected(Full) */ }
#[test] fn reliable_messages_arrive_in_order_despite_losses() { /* 30% потерь, перестановки: 1000 сообщений — все, по порядку */ }
#[test] fn unreliable_messages_may_be_lost_but_never_duplicated_to_the_game() { /* ... */ }
#[test] fn snapshots_drop_stale_packets() { /* прибыл номер 5 после 6: игнор */ }
#[test] fn a_message_roundtrips_through_postcard_with_its_id() { /* ... */ }
#[test] fn an_oversized_or_garbled_packet_is_rejected_without_a_panic() { /* фаззинг 1000 случайных байтовых массивов -> Err/ignore */ }
#[test] fn the_server_tick_estimate_converges_to_the_true_tick() { /* RTT 80 мс, джиттер ±10 мс: ошибка ≤ 1 тик после 2 с */ }
#[test] fn a_disconnect_is_reported_when_packets_stop() { /* таймаут 5 с */ }
```
Реальная сеть (интеграционный, `tests/udp_loopback.rs`, `#[ignore]` если порт занят): сервер + клиент на 127.0.0.1, обмен сообщениями, отключение.

## Шаг 3 — подключение к приложению и документация (S)
- `NetPlugin` в `redox_core` (по образцу `NavPlugin`): ресурсы `NetServer`/`NetClient`, системы `net_poll_system` в начале кадра и `net_flush_system` в конце; режимы `NetMode::{Off, Server, Client, ListenServer}` из `AppConfig`/аргументов (`--host`, `--connect addr`).
- Пример `crates/redox_core/examples/net_chat.rs`: консольный/egui-чат — сервер, клиенты, сообщения «Reliable»; режим автотеста `--selftest` (сервер+2 клиента в процессе, 10 сообщений, выход 0/1).
- Документы: `docs/adr/0008-networking.md` (статус → «принят» после подтверждения), `PLAN_TECH_OVERVIEW.md` (раздел «Сеть»), `PLAN_EXECUTION.md` (новая строка), `CHANGELOG.md`, `docs/CRATES.md` (регенерация).

## Границы
- **Читать:** `redox_core/src/plugin.rs`, `redox_nav` (`NavPlugin` как образец плагина), `Cargo.toml` workspace (зависимости), `deny.toml`, `docs/adr/0007-*.md` (формат ADR).
- **Создать:** `crates/redox_net/` (Cargo.toml, `src/{lib,transport,loopback,server,client,message,tick}.rs`, `examples/net_spike.rs`, `tests/`), `docs/adr/0008-networking.md`, `docs/engineering/notes/net-spike.md`, `redox_core/examples/net_chat.rs`.
- **Менять:** workspace `Cargo.toml` (член и зависимости), `deny.toml` (если нужна лицензия), `redox_core/src/plugin.rs`.
- **Не трогать:** ECS, физику.

## Проверка
`cargo fmt`, `cargo clippy -p redox_net -p redox_core --all-targets -- -D warnings`, `cargo test -p redox_net`, `cargo deny check`, `cargo check --workspace --all-targets`, `python scripts/check_file_sizes.py`, `python scripts/gen_crates_doc.py --check`.

## Для исполнителя
`git merge --ff-only main`; `Cargo.lock` из main. Выполни **только шаг 1**, остановись и отчитайся таблицей и черновиком ADR. Шаги 2–3 — после записи координатора «ADR подтверждён» в строке «Статус». Новые зависимости без записи в ADR не добавлять.
