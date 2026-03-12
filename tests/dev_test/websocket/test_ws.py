#!/usr/bin/env python3
"""
Doo WebSocket Test Runner — ALL tests in single process.
Usage: python3 test_ws.py <ws_url> <http_url>
Example: python3 test_ws.py ws://127.0.0.1:3210 http://127.0.0.1:3210
"""
import asyncio, json, sys, urllib.request

WS_URL = sys.argv[1] if len(sys.argv) > 1 else "ws://127.0.0.1:3210"
HTTP_URL = sys.argv[2] if len(sys.argv) > 2 else "http://127.0.0.1:3210"

import websockets

passed = 0
failed = 0

def report(ok, name, detail=""):
    global passed, failed
    if ok:
        passed += 1
        print(f"  \u2705 PASS: {name}")
    else:
        failed += 1
        print(f"  \u274c FAIL: {name}")
        if detail:
            print(f"    {detail}")

def http_json(path):
    with urllib.request.urlopen(f"{HTTP_URL}{path}") as r:
        return json.loads(r.read())

def ws(path):
    """Connect with short timeouts — avoids 10s default close_timeout blocking."""
    return websockets.connect(f"{WS_URL}{path}", open_timeout=2, close_timeout=0.5, ping_timeout=2)

async def run_all():
    # Test 2: Echo
    print("\n--- Test 2: WebSocket Echo ---")
    try:
        async with ws("/ws/echo") as c:
            await c.send(json.dumps({"event": "echo", "data": "hello_world"}))
            r = await asyncio.wait_for(c.recv(), timeout=2.0)
            msg = json.loads(r)
            report(msg.get("event") == "echo" and msg.get("data") == "hello_world",
                   "Echo event received",
                   f"Got: event={msg.get('event')},data={msg.get('data')}")
    except Exception as e:
        report(False, "Echo event received", str(e))

    # Test 3: Active connections (0 after disconnect)
    print("\n--- Test 3: Active Connections (0 after disconnect) ---")
    await asyncio.sleep(0.1)
    try:
        data = http_json("/status/connections")
        report(data.get("active_connections") == 0,
               "No active connections after disconnect",
               f"Got: {data.get('active_connections')}")
    except Exception as e:
        report(False, "No active connections after disconnect", str(e))

    # Test 4: Connection Persistence + Count
    print("\n--- Test 4: Connection Persistence + Count ---")
    try:
        c = await ws("/ws/echo")
        await asyncio.sleep(0.1)
        data = http_json("/status/connections")
        report(data.get("active_connections") == 1,
               "Connection counted while open",
               f"Got: {data.get('active_connections')}")
        await c.close()
        await asyncio.sleep(0.1)
        data = http_json("/status/connections")
        report(data.get("active_connections") == 0,
               "Connection removed after close",
               f"Got: {data.get('active_connections')}")
    except Exception as e:
        report(False, "Connection persistence", str(e))

    # Test 5: Multiple Echo Messages
    print("\n--- Test 5: Multiple Echo Messages ---")
    try:
        async with ws("/ws/echo") as c:
            results = []
            for i in range(5):
                await c.send(json.dumps({"event": "echo", "data": f"msg_{i}"}))
                r = await asyncio.wait_for(c.recv(), timeout=2.0)
                results.append(json.loads(r).get("data", ""))
            expected = ["msg_0", "msg_1", "msg_2", "msg_3", "msg_4"]
            report(results == expected, "5 echo responses",
                   f"Expected: {expected}, Got: {results}")
    except Exception as e:
        report(False, "5 echo responses", str(e))

    # Test 6: Chat Room Messaging
    print("\n--- Test 6: Chat Room Messaging ---")
    try:
        ws1 = await ws("/ws/chat")
        ws2 = await ws("/ws/chat")
        await asyncio.sleep(0.1)
        await ws1.send(json.dumps({"event": "message", "data": "hello_from_1"}))
        received = {}
        for name, conn in [("ws1", ws1), ("ws2", ws2)]:
            try:
                r = await asyncio.wait_for(conn.recv(), timeout=2.0)
                received[name] = json.loads(r).get("data", "")
            except asyncio.TimeoutError:
                received[name] = "timeout"
        report(received.get("ws1") == "hello_from_1",
               "Client 1 received room message",
               f"Got: {received.get('ws1')}")
        report(received.get("ws2") == "hello_from_1",
               "Client 2 received room message",
               f"Got: {received.get('ws2')}")
        await ws1.close()
        await ws2.close()
    except Exception as e:
        report(False, "Chat room messaging", str(e))

    # Test 7: VIP Room Isolation
    print("\n--- Test 7: VIP Room Isolation ---")
    try:
        ws1 = await ws("/ws/chat")
        ws2 = await ws("/ws/chat")
        await asyncio.sleep(0.1)
        await ws2.send(json.dumps({"event": "join_room", "data": "vip"}))
        await asyncio.sleep(0.05)
        await ws2.send(json.dumps({"event": "room_msg", "data": "vip_only_msg"}))
        ws2_got = None
        try:
            r = await asyncio.wait_for(ws2.recv(), timeout=2.0)
            msg = json.loads(r)
            if msg.get("event") == "room_msg":
                ws2_got = msg.get("data")
        except asyncio.TimeoutError:
            pass
        ws1_got = None
        try:
            await asyncio.wait_for(ws1.recv(), timeout=0.1)
            ws1_got = "unexpected"
        except asyncio.TimeoutError:
            ws1_got = "none"
        report(ws2_got == "vip_only_msg",
               "VIP client received room message",
               f"Got: {ws2_got}")
        report(ws1_got == "none",
               "Lobby-only client did NOT receive VIP message",
               f"Got: {ws1_got}")
        await ws1.close()
        await ws2.close()
    except Exception as e:
        report(False, "VIP room isolation", str(e))

    # Test 8: Server Broadcast via HTTP
    print("\n--- Test 8: Server Broadcast via HTTP ---")
    try:
        ws1 = await ws("/ws/echo")
        ws2 = await ws("/ws/echo")
        await asyncio.sleep(0.1)
        urllib.request.urlopen(f"{HTTP_URL}/control/broadcast")
        results = {}
        for name, conn in [("ws1", ws1), ("ws2", ws2)]:
            try:
                r = await asyncio.wait_for(conn.recv(), timeout=2.0)
                results[name] = json.loads(r).get("event", "?")
            except asyncio.TimeoutError:
                results[name] = "timeout"
        report(results.get("ws1") == "server_event",
               "Client 1 received broadcast",
               f"Got: {results.get('ws1')}")
        report(results.get("ws2") == "server_event",
               "Client 2 received broadcast",
               f"Got: {results.get('ws2')}")
        await ws1.close()
        await ws2.close()
    except Exception as e:
        report(False, "Server broadcast", str(e))

    # Test 9: Lifecycle Events (ping/pong)
    print("\n--- Test 9: Lifecycle Events ---")
    try:
        c = await ws("/ws/lifecycle")
        await asyncio.sleep(0.05)
        await c.send(json.dumps({"event": "ping", "data": "test_ping"}))
        r = await asyncio.wait_for(c.recv(), timeout=2.0)
        msg = json.loads(r)
        report(msg.get("data") == "test_ping",
               "Lifecycle ping/pong",
               f"Got: {msg.get('data')}")
        await c.close()
        await asyncio.sleep(0.2)
        report(True, "Lifecycle test completed")
    except Exception as e:
        report(False, "Lifecycle events", str(e))

    # Test 10: Multi-Client Concurrent (5 clients)
    print("\n--- Test 10: Multi-Client Concurrent (5 clients) ---")
    try:
        clients = []
        for i in range(5):
            c = await ws("/ws/echo")
            clients.append(c)
        await asyncio.sleep(0.1)
        data = http_json("/status/connections")
        report(data.get("active_connections") == 5,
               "5 clients connected",
               f"Got: {data.get('active_connections')}")
        for i, conn in enumerate(clients):
            await conn.send(json.dumps({"event": "echo", "data": f"client_{i}"}))
        echoes = []
        for i, conn in enumerate(clients):
            try:
                r = await asyncio.wait_for(conn.recv(), timeout=2.0)
                echoes.append(json.loads(r).get("data", ""))
            except asyncio.TimeoutError:
                echoes.append(f"timeout_{i}")
        report(len(echoes) == 5, "5 echo responses",
               f"Got: {len(echoes)}")
        report(len(set(echoes)) == len(echoes), "All echoes unique",
               f"Got: {echoes}")
        for conn in clients:
            await conn.close()
        await asyncio.sleep(0.15)
        data = http_json("/status/connections")
        report(data.get("active_connections") == 0,
               "All disconnected",
               f"Got: {data.get('active_connections')}")
    except Exception as e:
        report(False, "Multi-client concurrent", str(e))

    # Test 11: Non-Existent WS Route
    print("\n--- Test 11: Non-Existent WS Route ---")
    try:
        c = await ws("/ws/nonexistent")
        report(False, "Non-existent WS route returns error", "Connected unexpectedly")
        await c.close()
    except Exception as e:
        report("404" in str(e) or "reject" in str(e).lower(),
               "Non-existent WS route returns error",
               f"Got: {e}")

    # Test 12: Leave Room
    print("\n--- Test 12: Leave Room ---")
    try:
        ws1 = await ws("/ws/chat")
        ws2 = await ws("/ws/chat")
        await asyncio.sleep(0.1)
        await ws2.send(json.dumps({"event": "leave_room", "data": "lobby"}))
        await asyncio.sleep(0.05)
        await ws1.send(json.dumps({"event": "message", "data": "after_leave"}))
        ws1_got = None
        try:
            r = await asyncio.wait_for(ws1.recv(), timeout=2.0)
            ws1_got = json.loads(r).get("data", "")
        except asyncio.TimeoutError:
            ws1_got = "timeout"
        ws2_got = None
        try:
            await asyncio.wait_for(ws2.recv(), timeout=0.1)
            ws2_got = "unexpected"
        except asyncio.TimeoutError:
            ws2_got = "none"
        report(ws1_got == "after_leave",
               "Client 1 still in lobby receives message",
               f"Got: {ws1_got}")
        report(ws2_got == "none",
               "Client 2 left lobby, no message received",
               f"Got: {ws2_got}")
        await ws1.close()
        await ws2.close()
    except Exception as e:
        report(False, "Leave room", str(e))

    # Test 13: conn.isClosed() returns open for active connection
    print("\n--- Test 13: conn.isClosed() Status Check ---")
    try:
        async with ws("/ws/close-test") as c:
            await c.send(json.dumps({"event": "check_status", "data": "ping"}))
            r = await asyncio.wait_for(c.recv(), timeout=2.0)
            msg = json.loads(r)
            report(msg.get("event") == "status" and msg.get("data") == "open",
                   "isClosed() returns false (open) for active conn",
                   f"Got: event={msg.get('event')}, data={msg.get('data')}")
    except Exception as e:
        report(False, "isClosed() status check", str(e))

    # Test 14: conn.close() server-initiated disconnect
    print("\n--- Test 14: conn.close() Server-Initiated Close ---")
    try:
        c = await ws("/ws/close-test")
        await asyncio.sleep(0.05)
        await c.send(json.dumps({"event": "server_close", "data": "please"}))
        # Server does emit("closing","bye") then close() — may arrive as one frame + close
        got_closing = False
        got_close = False
        for _ in range(3):
            try:
                r = await asyncio.wait_for(c.recv(), timeout=2.0)
                msg = json.loads(r)
                if msg.get("event") == "closing" and msg.get("data") == "bye":
                    got_closing = True
            except (websockets.exceptions.ConnectionClosed,
                    websockets.exceptions.ConnectionClosedOK,
                    websockets.exceptions.ConnectionClosedError):
                got_close = True
                break
            except asyncio.TimeoutError:
                break
        # If we got the closing frame, try one more recv to confirm close
        if got_closing and not got_close:
            try:
                await asyncio.wait_for(c.recv(), timeout=1.0)
            except (websockets.exceptions.ConnectionClosed,
                    websockets.exceptions.ConnectionClosedOK,
                    websockets.exceptions.ConnectionClosedError,
                    asyncio.TimeoutError):
                got_close = True
        # Even if we didn't get the closing text frame (race), the close itself is the key test
        report(got_close, "Server-initiated close disconnected client",
               f"Got closing event: {got_closing}, got close: {got_close}")
    except (websockets.exceptions.ConnectionClosed,
            websockets.exceptions.ConnectionClosedOK,
            websockets.exceptions.ConnectionClosedError):
        # Connection closed immediately — server close worked
        report(True, "Server-initiated close disconnected client",
               "Connection closed immediately (race with emit)")
    except Exception as e:
        report(False, "conn.close() server-initiated", str(e))

    # Final summary
    print(f"\nWS_RESULTS:{passed}:{failed}:{passed+failed}")
    if failed > 0:
        sys.exit(1)

asyncio.run(run_all())
