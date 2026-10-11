// SPDX-License-Identifier: MIT OR Apache-2.0
// @category WonderSwan.Assets
import jidoughost.wonderswan.WSMachine;
import ghidra.program.model.address.*;
import ghidra.program.model.pcode.*;
import java.nio.file.*;
import java.io.*;
import java.util.*;
public class WSAssetTraceCarryChecks extends WSAssetTrace {
 @Override public void run() throws Exception {
  Path out=Paths.get(getScriptArgs()[0]);Files.createDirectories(out);
  machine=new WSMachine(currentProgram,true);
  accesses=new PrintWriter(new StringWriter());
  AddressSpace u=currentProgram.getAddressFactory().getUniqueSpace(),k=currentProgram.getAddressFactory().getConstantSpace();
  Varnode value=new Varnode(u.getAddress(256),2);put(u,256,2,Set.of(123L));
  Call call=new Call();stack.add(call);List<String> failures=new ArrayList<>();
  PcodeOp store=new PcodeOp(machine.ram.getAddress(0),0,PcodeOp.STORE,new Varnode[]{new Varnode(k.getAddress(machine.ram.getSpaceID()),4),new Varnode(k.getAddress(8192),4),value},null);
  stored(store,machine.ram,new byte[]{0,32},2,new byte[]{1,2});
  if(!call.sources.contains(123L))failures.add("entry register origins carried into callee store");
  call.sources.clear();
  PcodeOp load=new PcodeOp(machine.ram.getAddress(0),0,PcodeOp.LOAD,new Varnode[]{new Varnode(k.getAddress(machine.ram.getSpaceID()),4),new Varnode(k.getAddress(8192),4)},value);
  loaded(load,machine.ram,new byte[]{0,32},2);
  if(!call.sources.contains(123L))failures.add("earlier ROM origins carried through RAM load");
  call.sources.clear();machine.ports[0x48]=0;
  machine.dmaLog.add("{\"src\":8192,\"dst\":16384,\"words\":1}");dma();
  if(!call.sources.contains(123L) || !get(machine.ram,16384,2).equals(Set.of(123L)))failures.add("RAM-to-RAM DMA retains original ROM origins");
  machine=new WSMachine(currentProgram,false);call.sources.clear();put(u,256,2,Set.of(789L));
  stored(store,machine.ram,new byte[]{0,96},2,new byte[]{1,2});
  loaded(load,machine.ram,new byte[]{0,32},2);
  if(!get(u,256,2).equals(Set.of(789L)))failures.add("monochrome RAM mirrors share provenance cells");
  machine=new WSMachine(currentProgram,true);put(machine.ram,0,1,Set.of(999L));
  stored(store,machine.ram,new byte[]{(byte)255,(byte)255},2,new byte[]{1,2});
  if(!get(machine.ram,0,1).equals(Set.of(999L)))failures.add("CPU stores crossing 64 KiB do not wrap into RAM");
  StringWriter captured=new StringWriter();calls=new PrintWriter(captured);
  call.sources.clear();call.sources.addAll(Set.of(123L,2000L));call.regs="{}";
  put(machine.ram,8192,8,Set.of(123L));finishSpan(call,true,8192,8200);calls.flush();
  if(!captured.toString().contains("\"source_ranges\":[[123,124]]"))failures.add("witness lists only actual causal ROM ranges separately from read context");
  Files.writeString(out.resolve("checks.json"),"{\"checks\":6,\"failures\":"+listStrings(failures)+"}");
  println("CARRY TESTS: "+failures);
  if(!failures.isEmpty())throw new IllegalStateException("carry tests failed");
 }
}
