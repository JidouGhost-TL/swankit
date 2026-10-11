// SPDX-License-Identifier: MIT OR Apache-2.0
// Instruction-level provenance using the installed extension's own emulator.
// Args: output-directory frames input-script-or-dash [cycle-timing 0|1]
// @category WonderSwan.Assets
import ghidra.app.script.GhidraScript;
import ghidra.pcode.emu.*;
import ghidra.pcode.exec.*;
import ghidra.pcode.exec.PcodeExecutorStatePiece.Reason;
import ghidra.program.model.address.*;
import ghidra.program.model.listing.Instruction;
import ghidra.program.model.pcode.*;
import ghidra.program.model.scalar.Scalar;
import jidoughost.wonderswan.WSMachine;
import java.lang.reflect.*;
import java.nio.file.*;
import java.io.*;
import java.util.*;

public class WSAssetTrace extends GhidraScript {
    static final long IMMEDIATE = 1L << 40;
    static final int LIMIT = 64;
    static final Set<Long> NONE = Collections.emptySet();
    WSMachine machine;
    PrintWriter calls, accesses;
    int frame, nextId;
    long pc, ops, loads, stores, opaque;
    int dmaSeen;
    boolean pendingCall;
    long pendingReturn;
    int pendingSp;
    Instruction instruction;
    final Map<String, Set<Long>> cells = new HashMap<>();
    final Map<Long, Set<Long>> immediates = new HashMap<>();
    final List<Call> stack = new ArrayList<>();
    final Map<Integer, Set<Long>> portOrigins = new HashMap<>();

    static class Call {
        int id, frame, sp;
        long entry, ret;
        String regs;
        final TreeSet<Long> sources = new TreeSet<>(), pointers = new TreeSet<>();
        final TreeSet<Integer> destinations = new TreeSet<>();
        final Map<Integer,Integer> cpuDestinations = new HashMap<>();
        final TreeSet<String> banks = new TreeSet<>();
        long opaque;
    }
    static String key(AddressSpace space, long at) { return space.getSpaceID()+":"+at; }
    int physicalRam(long at) {return (int)at & (machine.color?65535:16383);}
    static long number(byte[] b) {
        long v=0; for(int i=0;i<Math.min(8,b.length);i++) v|=(b[i]&255L)<<(8*i); return v;
    }
    Set<Long> get(AddressSpace s, long at, int n) {
        TreeSet<Long> out=new TreeSet<>();
        for(int i=0;i<n;i++) out.addAll(cells.getOrDefault(key(s,at+i),NONE));
        return bounded(out);
    }
    Set<Long> bounded(TreeSet<Long> s) {
        if(s.size()>LIMIT) { opaque++; return NONE; }
        return s.isEmpty()?NONE:Collections.unmodifiableSet(s);
    }
    Set<Long> origins(Varnode n) {
        if(n.isConstant()) return immediates.getOrDefault(n.getOffset(),NONE);
        return get(n.getAddress().getAddressSpace(),n.getOffset(),n.getSize());
    }
    void testOp(int opcode, Varnode out, Varnode... in) {
        op(null,new PcodeOp(machine.ram.getAddress(0),0,opcode,in,out));
    }
    void selfTest(Path out) throws Exception {
        AddressSpace u=currentProgram.getAddressFactory().getUniqueSpace();
        AddressSpace k=currentProgram.getAddressFactory().getConstantSpace();
        Varnode a=new Varnode(u.getAddress(0x100),2),b=new Varnode(u.getAddress(0x200),4);
        List<String> failures=new ArrayList<>();
        put(u,0x100,2,Set.of(123L));put(u,0x102,2,Set.of(789L));
        testOp(PcodeOp.INT_ZEXT,b,a);
        if(!get(u,0x200,1).equals(Set.of(123L)) || !get(u,0x202,2).isEmpty())failures.add("zero extension preserves lower-byte origins and clears upper bytes");
        testOp(PcodeOp.INT_SEXT,b,a);
        if(!get(u,0x202,2).equals(Set.of(123L)))failures.add("sign extension uses sign-byte origins");
        testOp(PcodeOp.SUBPIECE,a,b,new Varnode(k.getAddress(1),4));
        if(!get(u,0x100,2).equals(Set.of(123L)))failures.add("subpiece and alias-safe copies");
        testOp(PcodeOp.INT_XOR,a,a,a);
        if(!get(u,0x100,2).isEmpty())failures.add("zero idiom clears provenance");
        put(u,0x200,2,Set.of(456L));immediates.put(7L,Set.of(IMMEDIATE+42));
        testOp(PcodeOp.PIECE,b,new Varnode(k.getAddress(7),2),new Varnode(u.getAddress(0x200),2));
        if(!get(u,0x200,2).equals(Set.of(456L)) || !get(u,0x202,2).equals(Set.of(IMMEDIATE+42)))failures.add("piece handles aliasing and immediate origins");
        testOp(PcodeOp.FLOAT_ADD,b,b,b);
        if(!get(u,0x200,4).isEmpty())failures.add("unsupported operation clears provenance");
        TreeSet<Long> many=new TreeSet<>();for(long i=0;i<=LIMIT;i++)many.add(i);
        if(!bounded(many).isEmpty())failures.add("origin overflow is opaque");
        int segment=-1;for(int i=0;i<machine.lang.getNumberOfUserDefinedOpNames();i++)if(machine.lang.getUserDefinedOpName(i).equals("segment"))segment=i;
        put(u,0x200,4,Set.of(456L));
        testOp(PcodeOp.CALLOTHER,a,new Varnode(k.getAddress(segment),4),b,new Varnode(k.getAddress(7),2));
        if(!get(u,0x100,2).equals(Set.of(456L,IMMEDIATE+42)))failures.add("documented segment translation retains offset and bank origins");
        Files.writeString(out.resolve("self-test.json"),"{\"checks\":8,\"failures\":"+listStrings(failures)+"}");
        println("PROVENANCE SELF TEST: 8 checks, "+failures.size()+" failures "+failures);
        if(!failures.isEmpty())throw new IllegalStateException("provenance self test failed");
        cells.clear();immediates.clear();ops=opaque=0;
    }
    void put(AddressSpace s, long at, int n, Set<Long> source) {
        for(int i=0;i<n;i++) {
            String k=key(s,at+i);
            if(source.isEmpty()) cells.remove(k); else cells.put(k,source);
        }
    }
    void copy(Varnode out, Varnode in, int skip) {
        List<Set<Long>> saved=new ArrayList<>();
        for(int i=0;i<out.getSize();i++) saved.add(in.isConstant()?origins(in):get(in.getAddress().getAddressSpace(),in.getOffset()+skip+i,1));
        for(int i=0;i<saved.size();i++) put(out.getAddress().getAddressSpace(),out.getOffset()+i,1,saved.get(i));
    }
    byte[] value(PcodeThread<byte[]> t, Varnode n) {
        return t.getState().getVar(n,Reason.INSPECT);
    }
    void op(PcodeThread<byte[]> t, PcodeOp op) {
        ops++;
        int code=op.getOpcode();Varnode out=op.getOutput();
        if(out==null || code==PcodeOp.LOAD) return;
        if(code==PcodeOp.COPY || code==PcodeOp.INT_ZEXT || code==PcodeOp.INT_SEXT) {
            Varnode in=op.getInput(0);
            if(in.getSize()==out.getSize()) {copy(out,in,0);return;}
            List<Set<Long>> saved=new ArrayList<>();
            for(int i=0;i<out.getSize();i++) {
                if(i<in.getSize())saved.add(in.isConstant()?origins(in):get(in.getAddress().getAddressSpace(),in.getOffset()+i,1));
                else if(code==PcodeOp.INT_SEXT)saved.add(in.isConstant()?origins(in):get(in.getAddress().getAddressSpace(),in.getOffset()+in.getSize()-1,1));
                else saved.add(NONE);
            }
            for(int i=0;i<saved.size();i++)put(out.getAddress().getAddressSpace(),out.getOffset()+i,1,saved.get(i));
            return;
        }
        if(code==PcodeOp.SUBPIECE) {copy(out,op.getInput(0),(int)op.getInput(1).getOffset());return;}
        if(code==PcodeOp.PIECE) {
            Varnode hi=op.getInput(0),lo=op.getInput(1);
            List<Set<Long>> saved=new ArrayList<>();
            for(int i=0;i<lo.getSize();i++) saved.add(lo.isConstant()?origins(lo):get(lo.getAddress().getAddressSpace(),lo.getOffset()+i,1));
            for(int i=0;i<hi.getSize();i++) saved.add(hi.isConstant()?origins(hi):get(hi.getAddress().getAddressSpace(),hi.getOffset()+i,1));
            for(int i=0;i<saved.size();i++) put(out.getAddress().getAddressSpace(),out.getOffset()+i,1,saved.get(i));
            return;
        }
        if((code==PcodeOp.INT_XOR || code==PcodeOp.INT_SUB) && op.getInput(0).equals(op.getInput(1))) {
            put(out.getAddress().getAddressSpace(),out.getOffset(),out.getSize(),NONE);return;
        }
        TreeSet<Long> sources=new TreeSet<>();
        // Integer and boolean operators propagate data dependencies. Branch
        // conditions are not blanket taint: independent later writes stay independent.
        boolean segment=code==PcodeOp.CALLOTHER && machine.lang.getUserDefinedOpName((int)op.getInput(0).getOffset()).equals("segment");
        if((code>=PcodeOp.INT_EQUAL && code<=PcodeOp.BOOL_OR) || code==PcodeOp.COPY || segment) {
            for(int i=segment?1:0;i<op.getNumInputs();i++) sources.addAll(origins(op.getInput(i)));
        } else { opaque++; }
        put(out.getAddress().getAddressSpace(),out.getOffset(),out.getSize(),bounded(sources));
    }
    String regs() {
        StringJoiner j=new StringJoiner(",","{","}");
        for(String name:List.of("AX","BX","CX","DX","SI","DI","BP","SP","CS","DS","ES","SS"))
            j.add("\""+name+"\":"+machine.reg(machine.lang.getRegister(name)));
        return j.toString();
    }
    void before(Instruction ins) throws Exception {
        instruction=ins;pc=ins.getAddress().getOffset();
        int sp=(int)machine.reg(machine.lang.getRegister("SP"));
        while(!stack.isEmpty()) {
            Call c=stack.get(stack.size()-1);
            if(pc!=c.ret || sp!=c.sp) break;
            finish(c,true);stack.remove(stack.size()-1);
        }
        if(pendingCall) {
            if(stack.size()<128) {
                Call c=new Call();c.id=nextId++;c.entry=pc;c.ret=pendingReturn;c.sp=pendingSp;c.regs=regs();c.frame=frame;c.opaque=opaque;stack.add(c);
            }
            pendingCall=false;
        }
        immediates.clear();
        byte[] bytes=ins.getParsedBytes();
        for(int operand=0;operand<ins.getNumOperands();operand++) {
            for(Object ob:ins.getOpObjects(operand)) if(ob instanceof Scalar scalar) {
                long v=scalar.getUnsignedValue();int n=(scalar.bitLength()+7)/8;
                if(n>4) continue;
                TreeSet<Long> matched=new TreeSet<>();int matches=0;
                for(int at=1;at+n<=bytes.length;at++) {
                    boolean same=true;
                    for(int k=0;k<n;k++) if((bytes[at+k]&255)!=((v>>>(8*k))&255)) same=false;
                    if(same) {matches++;long ro=machine.romOffset(pc+at);if(ro>=0) for(int k=0;k<n;k++) matched.add(IMMEDIATE+ro+k);}
                }
                if(matches==1) immediates.put(v,bounded(matched));
            }
        }
    }
    void loaded(PcodeOp op, AddressSpace space, byte[] offset, int size) {
        if(space!=machine.ram) {put(op.getOutput().getAddress().getAddressSpace(),op.getOutput().getOffset(),size,NONE);return;}
        loads++;long at=number(offset);
        Varnode out=op.getOutput();
        Set<Long> ptr=origins(op.getInput(1));
        TreeSet<Long> all=new TreeSet<>();
        for(int i=0;i<size;i++) {
            long ro=machine.romOffset(at+i);
            Set<Long> src=ro>=0?Set.of(ro):get(space,at+i<65536?physicalRam(at+i):at+i,1);
            all.addAll(src);put(out.getAddress().getAddressSpace(),out.getOffset()+i,1,src);
        }
        for(Call c:stack) {
            for(long ro:all) if(ro<IMMEDIATE)c.sources.add(ro);
            c.pointers.addAll(ptr);
            if(at>=0x20000)c.banks.add(machine.ports[0xc0]+":"+machine.ports[0xc2]+":"+machine.ports[0xc3]);
        }
    }
    void stored(PcodeOp op, AddressSpace space, byte[] offset, int size, byte[] value) {
        long at=number(offset);Varnode src=op.getInput(2);
        if(space==machine.io) {
            for(int i=0;i<size;i++)portOrigins.put(((int)at+i)&255,origins(src));
            if((at&255)==0x48 && (number(value)&0x80)!=0) dma();
            return;
        }
        if(space!=machine.ram || at>=0x10000)return;
        stores++;
        for(int i=0;i<size;i++) {
            if(at+i>=65536)continue;
            int physical=physicalRam(at+i);
            Set<Long> origin=src.isConstant()?origins(src):get(src.getAddress().getAddressSpace(),src.getOffset()+i,1);
            put(space,physical,1,origin);
            for(Call c:stack) {
                for(long source:origin)if(source<IMMEDIATE)c.sources.add(source);
                c.destinations.add(physical);c.cpuDestinations.put(physical,(int)at+i);
            }
        }
    }
    void dma() {
        while(dmaSeen<machine.dmaLog.size()) {
            String event=machine.dmaLog.get(dmaSeen++);
            accesses.println(event);
            if(event.contains("refused"))continue;
            Map<String,Integer> fields=new HashMap<>();
            java.util.regex.Matcher matcher=java.util.regex.Pattern.compile("\"([a-z_]+)\":([0-9]+)").matcher(event);
            while(matcher.find())fields.put(matcher.group(1),Integer.parseInt(matcher.group(2)));
            int source=fields.get("src"),dest=fields.get("dst"),n=fields.get("words")*2;
            int step=(machine.ports[0x48]&0x40)!=0?-2:2;
            for(int i=0;i<n;i+=2)for(int b=0;b<2;b++) {
                int a=(source+(i/2)*step+b)&0xfffff,cpuDest=(dest+(i/2)*step+b)&65535,d=physicalRam(cpuDest);
                long ro=machine.romOffset(a);Set<Long> origin=ro>=0?Set.of(ro):get(machine.ram,a<65536?physicalRam(a):a,1);
                put(machine.ram,d,1,origin);
                for(Call c:stack) {
                    for(long sourceOrigin:origin)if(sourceOrigin<IMMEDIATE)c.sources.add(sourceOrigin);
                    c.destinations.add(d);c.cpuDestinations.put(d,cpuDest);
                }
            }
        }
    }
    void finish(Call c,boolean complete) {
        if(c.sources.isEmpty() || c.destinations.isEmpty())return;
        if(c.sources.size()>65536)return;
        int lo=-1, last=-1;
        for(int a:c.destinations) {
            if(lo<0) {lo=a;last=a;continue;}
            if(a>last+1) {finishSpan(c,complete,lo,last+1);lo=a;}
            last=a;
        }
        if(lo>=0)finishSpan(c,complete,lo,last+1);
    }
    void finishSpan(Call c, boolean complete, int lo, int hi) {
        if(hi-lo<8 || hi-lo>32768)return;
        // Explicitly list causal byte membership; holes are never called complete.
        TreeSet<Long> causal=new TreeSet<>();int explained=0;
        for(int a=lo;a<hi;a++) {
            Set<Long> s=get(machine.ram,a,1);
            boolean hit=false;for(long x:s)if(x<IMMEDIATE && c.sources.contains(x)){causal.add(x);hit=true;}
            if(hit)explained++;
        }
        if(causal.isEmpty())return;
        byte[] output=machine.read(lo,hi-lo);StringBuilder hex=new StringBuilder();for(byte b:output)hex.append(String.format("%02x",b&255));
        calls.printf("{\"kind\":\"instruction-flow\",\"id\":%d,\"frame\":%d,\"entry\":%d,\"return\":%d,\"entry_registers\":%s,\"source\":%d,\"source_end\":%d,\"source_bytes\":%d,\"causal_source_bytes\":%d,\"source_ranges\":%s,\"destination\":%d,\"cpu_destination\":%d,\"length\":%d,\"explained_bytes\":%d,\"complete_return\":%s,\"opaque_operations\":%d,\"pointer_origins\":%s,\"banks\":%s,\"output_hex\":\"%s\"}%n",c.id,c.frame,c.entry,c.ret,c.regs,c.sources.first(),c.sources.last()+1,c.sources.size(),causal.size(),sourceRanges(causal),lo,c.cpuDestinations.getOrDefault(lo,lo),hi-lo,explained,complete,opaque-c.opaque,c.pointers,listStrings(c.banks),hex);
    }
    static String sourceRanges(TreeSet<Long> sources) {
        StringJoiner ranges=new StringJoiner(",","[","]");long lo=-1,hi=-1;
        for(long source:sources) {
            if(lo<0) {lo=source;hi=source+1;}
            else if(source==hi)hi++;
            else {ranges.add("["+lo+","+hi+"]");lo=source;hi=source+1;}
        }
        if(lo>=0)ranges.add("["+lo+","+hi+"]");return ranges.toString();
    }
    static String listStrings(Collection<String> s) {StringJoiner j=new StringJoiner(",","[","]");for(String x:s)j.add("\""+x+"\"");return j.toString();}
    @Override public void run() throws Exception {
        String[] args=getScriptArgs();Path out=Paths.get(args[0]);Files.createDirectories(out);int frames=Integer.parseInt(args[1]);
        machine=new WSMachine(currentProgram,currentProgram.getOptions("WonderSwan").getBoolean("Color",true));
        if(args.length>4 && args[4].equals("self-test")) {selfTest(out);return;}
        machine.cycleTiming=args.length>3 && args[3].equals("1");machine.traceLimit=0;
        calls=new PrintWriter(out.resolve("calls.jsonl").toFile());accesses=new PrintWriter(out.resolve("dma.jsonl").toFile());
        // Preserve every hardware callback. Only this private instance's callback
        // field is wrapped; no installed extension or shared state is modified.
        Field field=AbstractPcodeMachine.class.getDeclaredField("cb");field.setAccessible(true);
        PcodeEmulationCallbacks<byte[]> original=(PcodeEmulationCallbacks<byte[]>)field.get(machine.emu);
        Object wrapper=Proxy.newProxyInstance(PcodeEmulationCallbacks.class.getClassLoader(),new Class[]{PcodeEmulationCallbacks.class},(proxy,method,a)->{
            if(method.getName().equals("beforeExecuteInstruction"))before((Instruction)a[1]);
            if(method.getName().equals("beforeStepOp"))op((PcodeThread<byte[]>)a[0],(PcodeOp)a[1]);
            Object result;
            try {result=method.invoke(original,a);}catch(InvocationTargetException e){throw e.getCause();}
            switch(method.getName()) {
                case "afterLoad" -> loaded((PcodeOp)a[1],(AddressSpace)a[2],(byte[])a[3],(int)a[4]);
                case "beforeStore" -> stored((PcodeOp)a[1],(AddressSpace)a[2],(byte[])a[3],(int)a[4],(byte[])a[5]);
                case "afterExecuteInstruction" -> {
                    dma(); Instruction ins=(Instruction)a[1];if(ins.getFlowType().isCall()) {
                        pendingCall=true;pendingReturn=ins.getAddress().getOffset()+ins.getLength();
                        int bytes=ins.getMnemonicString().equals("CALLF")?4:2;
                        pendingSp=((int)machine.reg(machine.lang.getRegister("SP"))+bytes)&65535;
                    }
                }
            }
            return result;
        });field.set(machine.emu,wrapper);
        List<int[]> keys=new ArrayList<>();if(!args[2].equals("-"))for(String line:Files.readAllLines(Paths.get(args[2]))) {
            line=line.replaceAll("#.*","").trim();if(line.isEmpty())continue;String[] k=line.split("\\s+");keys.add(new int[]{Integer.parseInt(k[0]),Integer.parseInt(k[1]),Integer.parseInt(k[2],16)});
        }
        String error=null;
        try {machine.run(frames,40000,f->{frame=f;machine.buttons=0;for(int[] k:keys)if(f>=k[0] && f<=k[1])machine.buttons|=k[2];});}
        catch(RuntimeException e){error=e.toString();println("TRACE ERROR "+error);}
        for(Call c:stack)finish(c,false);calls.close();accesses.close();
        Files.write(out.resolve("ram.bin"),machine.read(0,65536));byte[] ports=new byte[256];for(int i=0;i<256;i++)ports[i]=(byte)machine.ports[i];Files.write(out.resolve("ports.bin"),ports);
        Files.writeString(out.resolve("status.json"),String.format("{\"frames\":%d,\"instructions\":%d,\"pcode_ops\":%d,\"loads\":%d,\"stores\":%d,\"opaque\":%d,\"error\":%s}",frames,machine.instructions,ops,loads,stores,opaque,error==null?"null":"\""+error.replace("\"","'")+"\""));
        println("WSAssetTrace: instructions="+machine.instructions+" loads="+loads+" stores="+stores+" calls="+nextId+" opaque="+opaque);
    }
}
