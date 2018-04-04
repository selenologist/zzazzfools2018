// deobfuscated window.onSerialDataReceived
//
// I'm naming the states:
// 1 - SYNC   (because it appears to wait for a 3 byte handshake before advancing to GETLEN)
// 2 - GETLEN (because it gets the length of the request)
// 3 - GETREQ (because it gets the request)
// 4 - SEND   (because it sends the request to the server)
// 5 - POLL   (because it checks for the response from the server)
// 6 - RECV   (because it passes the response to the gameboy)

function() {
    evr = {
        hsc: 0,
        cl: 0,        // content length?
        rs: !1,
        b: [],        // data storage?
        st: 1,        // state initially 1
        la: +new Date // last time
    }, brC = !1, brS = !1, brD = [];

    function n(n) {
        var t = new XMLHttpRequest;
        t.open("POST", "http://167.99.192.164:12709/req/" + localStorage.d_sessid, !0), t.onreadystatechange = function() {
            // on response
            // if readyState == (DONE) {
            //   if status = (OK) {
            //     brD = decode(response)
            //     brC = true
            //     brS = true
            //   } else {
            //     brC = true
            //     brS = false
            //   }
            // }
            4 == t.readyState && (200 == t.status ? (brD = base64ToArray(t.responseText), brC = !0, brS = !0) : (brC = !0, brS = !1))
        },
        // brS = false, brC = false, send(request)
        brS = !1, brC = !1, t.send(arrayToBase64(n))
    }

    function t(e) {
        var t = 255,
            r = +new Date; // current unix time in millis
        // if time since last more than 2 seconds then set (state = 1, hsc = 0, b = [])
        // and always (last=now, state == 1) (if bool is "state == 1")
        if(r - evr.la > 2e3){
            evr.st = 1;
            evr.hsc = 0;
            evr.b = [];
        }
        evr.la = r;

        if(evr.st == 1){ // if state == 1 (SYNC)
            var s = [218, 207, 235]; // ordered response codes?
            if(s[evr.hsc] == e){
                t = [165, 90, 10][evr.hsc];
                evr.hsc += 1;
                if(evr.hsc >= s.length){
                    evr.hsc = 0;
                    evr.st = 2; // state 1->2
                }
            }
            else{
                t = 0;
                evr.hsc = 0;
            }
        }
        /* old
        if (r - evr.la > 2e3 && (evr.st = 1, evr.hsc = 0, evr.b = []), evr.la = r, evr.st == 1) {
            var s = [218, 207, 235];
            // if e == s[evr.hsc]
            s[evr.hsc] == e ?
                // then
                (t = [165, 90, 10][evr.hsc], evr.hsc += 1,
                    // if hsc >= 3     then hsc = 0,     state 1->2
                    evr.hsc >= s.length && (evr.hsc = 0, evr.st = 2)) :
                // else t=0, hsc = 0
                (t = 0, evr.hsc = 0)
        }*/
        else if (evr.st == 2) { // if state == 2 (GETLEN)
            evr.b.push(e);
            if(evr.b.length >= 2){
                // cl = little endian 16 bit from b
                evr.cl = evr.b[0] + 256 * evr.b[1];
                evr.st = 3; // state 2->3
                if(evr.cl > 1280){ // this looks like a size limit
                    // the original code pointlessly sets t=255 here, which is then never read from.
                    // return void(...); always returns undefined
                    return undefined; // this looks like an "invalid operation" marker
                }
            }
            t = 204;
            /* old partially deobfuscated:
            // if b.len >= 2 after adding e then set
            //                                      (cl = little-endian 16 bits in b,    state=3,    if bool is "cl > 1280")
            // basically "if(b.len >= 2 && cl > 1280) then { set t = 255 and return undefined } else { t = 204, return t }
            if (evr.b.push(e), evr.b.length >= 2 && (evr.cl = evr.b[0] + 256 * evr.b[1], evr.st = 3, evr.cl > 1280)) return void(t = 255);
            t = 204 */
        }
        else {
            if(evr.st == 3){ // if state = 3 (GETREQ)
                evr.b.push(e);
                if(evr.b.length >= evr.cl){
                    evr.st = 4; // state 3->4
                }
                t = 204;
            }
            else if(evr.st == 4){ // if state = 4 (SEND)
                if(e != 85){
                    evr.st = 1; // state 4->1
                }
                else{
                    n(evr.b); // send request from b
                    evr.st = 5; // state 4->5
                }
                t = 102;
            }
            else if(evr.st == 5){ // if state = 5 (POLL)
                if(e != 85){
                    evr.st = 1; // state 5->1
                }
                if(brC){
                    if(brS){
                        evr.b = brD; // b = decoded response data
                        evr.st = 6; // state 5->6
                        t = 51;
                    }
                    else{
                        t = 255;
                    }
                }
                else{
                    t = 102;
                }
            }
            else if(evr.st == 6){ // if state = 6 (RECV)
                t = evr.b[0]; // t is first byte of b
                evr.b = evr.b.slice(1); // b is b without that first byte (i.e. b shifted into t)
                if(evr.b.length <= 0 || // if b is now empty
                   204 != e){           // or e wasn't 204
                    evr.st = 1; // state 6->1
                }
            }
            return t;
            /* old partially deobfuscated:
            evr.st == 3 ? // if state == 3
                // then push e into b, if b's length is now >= cl then set state=4
                // always set t=204
                (evr.b.push(e), evr.b.length >= evr.cl && (evr.st = 4), t = 204) :
                // elseif state==4
                evr.st == 4 ?
                    // if e != 85
                    (85 != e ?
                        // state = 1
                        evr.st = 1 :
                        // else send request, state=5
                        (n(evr.b), evr.st = 5),
                        // always t=102
                        t = 102) :
                // elseif state==5
                evr.st == 5 ?
                    // if e != 85 then state=1
                    (85 != e && (evr.st = 1),
                    // always if brC
                    brC ?
                        // if brS
                        brS ?
                            // b=brD, state=6, t=51
                            (evr.b = brD, evr.st = 6, t = 51) :
                            // else t=255
                            t = 255 :
                        // else t = 102
                        t = 102) :
                // elseif state==6 then t=b[0], b=b[1:], if b is now zero or e != 204 then state = 1
                evr.st == 6 && (t = evr.b[0], evr.b = evr.b.slice(1), (evr.b.length <= 0 || 204 != e) && (evr.st = 1));
        return t*/
    };
    window.onSerialDataReceived = t
}
